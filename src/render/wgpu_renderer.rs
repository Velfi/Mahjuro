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
use crate::render::candle_mesh::{CandlePlacement, build_candle_wax_mesh, build_candle_wick_mesh};
use crate::render::coin_mesh::build_coin_mesh;
use crate::render::curio_cabinet_mesh::build_curio_cabinet_mesh;
use crate::render::decal::{
    LabelAlign, load_noto_emoji_font, load_ui_font, rasterize_label_styled_with_fallback,
    rasterize_tile_face_decal,
};
use crate::render::draw_cmd::{
    BookPlacement, BowlPlacement, CascadeTokenKind, CascadeTokenPlacement, CoinPlacement,
    CurioCabinetPlacement, DishExplicit, DrawCmd, ExtrudedGlyphPlacement, FallingBonePlacement,
    GoldBarPlacement, MirrorPlacement, OfudaPlacement, PackPlacement, PlaquePlacement,
    RelicPlacement, RelicShowcasePlacement, ShowcaseTilePlacement, ShrinePlacement,
    TallyFanKind, TallyFanPlacement, TalismanPlacement, UiFrame, WallStackPlacement,
    WoodTabletPlacement, YakuTabletPlacement, ZodiacRibbonPlacement,
};
use crate::render::gpu_types::{DecodedRelicImage, RelicTextureGpu};
use crate::render::lamp_mesh::{
    build_bug_body_mesh, build_bug_wing_mesh, build_lamp_body_mesh, build_lamp_bulb_mesh,
};
use crate::render::orb_mesh::build_orb_mesh;
use crate::render::lit_mesh::{
    LitMeshGpu, LitMeshInstance, MaterialKind, MaterialParams, ShadowCasterUniform, ShadowGlobals,
    SsrGlobals, create_lit_mesh_material_layout, create_lit_mesh_ssr_layout,
    create_shadow_caster_layout, create_shadow_sample_layout,
};
use crate::render::mirror_mesh::{MIRROR_LOCAL_CENTER_Y, MIRROR_LOCAL_HALF, build_mirror_mesh};
use crate::render::ofuda_mesh::build_ofuda_mesh;
use crate::render::tally_stick_mesh::{
    build_tally_stick_base_mesh, build_tally_stick_tip_mesh,
};
use crate::render::plaque_mesh::build_plaque_mesh;
use crate::render::relic_dish::{
    build_book_mesh, build_dish_mesh, build_pack_mesh, build_relic_mesh,
    build_relic_mesh_from_rgba, build_round_dish_mesh, build_shop_action_prop_mesh,
    build_tent_card_mesh,
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
    mesh_y_thickness_along_local_y_to_z_up, ribbon_submesh,
    rot_euler_xyz_rad, rot_rx_ry_rz_deg, rot_rx_rz_deg, rot_ry_rx_deg, rot_ry_rx_rz_rad,
    rot_rz_rx_deg, rot_rz_ry_rx_deg, rot_x_rad, rot_z_rad, rotation_around_point_x_rad,
    score_popup_glyph_rot_rad, table_mesh_lay_flat, tile_mesh_local_to_world, translate_rot_scale,
};
use crate::render::talisman_mesh::{TALISMAN_LOCAL_HALF, build_talisman_mesh, talisman_material};
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
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
}

/// Maximum number of point lights uploaded each frame. Must match the array
/// length in tile_3d.wgsl.
pub const MAX_POINT_LIGHTS: usize = 16;

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
/// running into `&mut self` conflicts with the frame-scoped `&self.scene_color_view`.
#[allow(clippy::too_many_arguments)]
fn ensure_lit_mesh_pool(
    pool: &mut Vec<LitMeshInstance>,
    n: usize,
    device: &wgpu::Device,
    material_layout: &wgpu::BindGroupLayout,
    shadow_caster_layout: &wgpu::BindGroupLayout,
    white_view: &wgpu::TextureView,
    relief_default_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) {
    while pool.len() < n {
        pool.push(LitMeshInstance::new(
            device,
            material_layout,
            shadow_caster_layout,
            white_view,
            relief_default_view,
            sampler,
        ));
    }
}

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
#[allow(dead_code)]
struct TilePrimitiveGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    albedo_texture: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    base_color_factor: [f32; 4],
}

/// A relic icon to draw as a textured quad at a screen-space rect.
pub struct RelicIcon {
    /// Position in screen pixels: [x, y, w, h].
    pub rect: [f32; 4],
    /// Which relic image to display.
    pub relic_id: crate::core::relic::RelicId,
}

/// Horizontal alignment of text inside its rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    #[allow(dead_code)]
    Right,
}

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign::Center
    }
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
#[allow(dead_code)]
struct HandTileGpu {
    /// Written every frame with view_proj + model + base_color_factor.
    uniform_buffer: wgpu::Buffer,
    /// One bind group per tile-mesh primitive.  Each binds the per-tile uniform
    /// + per-tile decal + that primitive's own albedo texture.
    bind_groups: Vec<wgpu::BindGroup>,
    /// Companion uniform buffer for the gold-metal outline shell. Written
    /// every frame the tile is *selected* with an inflated model matrix
    /// (uniform 1.06× scale around the tile center). Always allocated so
    /// the bind group can stay constant for the lifetime of the tile.
    outline_uniform_buffer: wgpu::Buffer,
    /// Bind groups that point at `outline_uniform_buffer` instead of the
    /// regular one. Same layout as `bind_groups`.
    outline_bind_groups: Vec<wgpu::BindGroup>,
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
    /// Main label (number or name) for the tile face.
    symbol: String,
    /// Emoji suit indicator rendered below the main label.
    suit_emoji: String,
    /// Suit colour for rendering the symbol (RGBA, linear).
    suit_color: [f32; 4],
    /// Kept alive so the GPU texture is not freed while bind_group references it.
    #[allow(dead_code)]
    decal_texture: wgpu::Texture,
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
    #[allow(dead_code)]
    decal_texture: wgpu::Texture,
}

const MAX_SHOWCASE_TILE_SLOTS: usize = 160;

// Tile-mesh local extents (after `normalize_mesh` in tile_glb.rs):
//   local X — long face axis  (extent ~1.000) → table-Z (front-back)
//   local Y — thickness        (extent ~0.424) → world Y (up off table)
//   local Z — short face axis  (extent ~0.734) → table-X (left-right)
const LOCAL_X_EXTENT: f32 = 1.000;
const LOCAL_Y_EXTENT: f32 = 0.424;
const LOCAL_Z_EXTENT: f32 = 0.734;

/// Camera state captured at the end of a frame, for unprojecting cursor
/// positions into world-space rays in `pick_hand_tile`.
#[derive(Clone, Copy)]
struct PickCamera {
    inv_view_proj: Mat4,
    viewport_w: f32,
    viewport_h: f32,
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

pub struct WgpuRenderer {
    surface: wgpu::Surface<'static>,
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
    flame_pipeline: wgpu::RenderPipeline,
    starfield_pipeline: wgpu::RenderPipeline,
    ember_drift_pipeline: wgpu::RenderPipeline,
    golden_dust_pipeline: wgpu::RenderPipeline,
    moonlit_water_pipeline: wgpu::RenderPipeline,
    // Owns the GPU resource that `moon_albedo_bind_group` samples from.
    #[allow(dead_code)]
    moon_albedo_texture: wgpu::Texture,
    moon_albedo_bind_group: wgpu::BindGroup,
    sunlit_water_pipeline: wgpu::RenderPipeline,
    shooting_star_cascade_pipeline: wgpu::RenderPipeline,
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
    last_pick_models: Vec<(usize, Mat4)>,
    /// Camera state captured at the end of the previous frame, used by
    /// `pick_hand_tile` to unproject the cursor into a world-space ray.
    last_pick_camera: Option<PickCamera>,
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
    #[allow(dead_code)] // Owns the GPU resource backing `lit_mesh_white_view`.
    lit_mesh_white_tex: wgpu::Texture,
    lit_mesh_white_view: wgpu::TextureView,
    /// Default `relief_tex` for lit meshes without a per-asset height map.
    #[allow(dead_code)] // Owns the GPU resource backing `lit_mesh_relief_default_view`.
    lit_mesh_relief_default_tex: wgpu::Texture,
    lit_mesh_relief_default_view: wgpu::TextureView,
    /// Linear-format heightmap texture for the shop coin faces. Bound at
    /// slot 1 of every coin instance — sampled by the metal branch in
    /// `lit_mesh.wgsl` to perturb the surface normal so the engraved
    /// Chinese cash-coin face catches the candle highlights. Kept on the
    /// renderer purely so the GPU resource outlives the bind groups that
    /// reference it.
    #[allow(dead_code)]
    lit_mesh_coin_height_tex: wgpu::Texture,
    // View is cloned into coin bind groups; field exists so the view outlives them.
    #[allow(dead_code)]
    lit_mesh_coin_height_view: wgpu::TextureView,
    /// Per-kind procedural heightmap textures for talisman tablets. Indexed
    /// by `TalismanKind::all()` order (see `talisman_height_paths` in `new`).
    /// The talisman shader branch samples these as a
    /// grayscale heightfield to perturb the surface normal.
    #[allow(dead_code)] // Backs `talisman_height_views`; kept alive for the views' lifetime.
    talisman_height_textures: Vec<wgpu::Texture>,
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
    /// Procedural dish mesh (a low brass tray shape) + per-relic placeholder
    /// mesh (a unit box). Both are stamped via lit-mesh instances.
    dish_mesh: LitMeshGpu,
    /// Round variant of `dish_mesh` for the sell tray and coin dish — circular
    /// rim + recessed floor instead of the square box.
    round_dish_mesh: LitMeshGpu,
    /// Folded "tent card" mesh sat on the sell-tray floor when focused; carries
    /// a "SELL" decal on each side via `sell_card_instance`.
    sell_card_mesh: LitMeshGpu,
    relic_box_mesh: LitMeshGpu,
    /// Unit box for tile booster packs (correct UVs per face; avoids the relic
    /// cylinder's repeated side strips).
    pack_mesh: LitMeshGpu,
    /// Rectangle for shop action props (Leave / Restock), decal on +Y face with
    /// U along +X so landscape labels render upright.
    shop_action_prop_mesh: LitMeshGpu,
    /// Per-relic silhouette-derived meshes generated from the loaded relic
    /// texture alpha. Falls back to `relic_box_mesh` when no usable silhouette
    /// can be derived.
    relic_meshes: HashMap<RelicId, LitMeshGpu>,
    /// Pre-allocated per-candle uniform buffers + bind groups (one per
    /// primitive). Indexed by candle slot, then 0=wax/1=wick.
    candle_instances: Vec<[LitMeshInstance; 2]>,
    /// Single uniform buffer + bind group for the gameplay-scene table.
    table_instance: LitMeshInstance,
    /// Single uniform buffer for the dish (sized + positioned per frame).
    dish_instance: LitMeshInstance,
    /// Pre-allocated per-relic-placeholder instances. Sized at startup to
    /// match `MAX_RELIC_SLOTS`; indexed by placement order each frame.
    relic_instances: Vec<LitMeshInstance>,
    /// Per-relic-placeholder world-space model matrices captured each frame
    /// for `pick_shop_object` raycasting.
    last_relic_models: Vec<Mat4>,
    /// Currently bound relic texture per slot. `Some(id)` means that slot's
    /// bind group already points at the texture for `id`; `None` means the
    /// flat-white fallback. Avoids rebuilding bind groups every frame.
    relic_slot_texture: Vec<Option<RelicId>>,
    /// Pre-allocated per-pack instances (lit-mesh foil; uses `pack_mesh` geometry).
    pack_instances: Vec<LitMeshInstance>,
    pack_slot_texture: Vec<Option<TilePackKind>>,
    // ── Shop scene meshes (curio cabinet + ribbons + talismans + coins) ─
    ribbon_mesh: LitMeshGpu,
    talisman_mesh: LitMeshGpu,
    coin_mesh: LitMeshGpu,
    cabinet_mesh: LitMeshGpu,
    /// Procedural shrine mesh used by the pick-blind scene.
    shrine_mesh: LitMeshGpu,
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
    /// Per-coin instances (shop scene). Indexed sequentially by `CoinBatch`
    /// placement order; truncated at `MAX_COIN_SLOTS`.
    coin_instances: Vec<LitMeshInstance>,
    /// Per-gold-bar instances (shop scene). Rendered as unit-box meshes
    /// with Metal material. Truncated at `MAX_BAR_SLOTS`.
    bar_instances: Vec<LitMeshInstance>,
    /// Procedural book mesh (rounded spine + page inset). Used by the shop
    /// scene for the Yaku Journal bookend.
    book_mesh: LitMeshGpu,
    /// Single instance for the journal book.
    book_instance: LitMeshInstance,
    /// Book model matrix + pick_id from the last frame, for slab-test picking.
    last_book_model: Option<(Mat4, u32)>,
    /// Up to 2 instances for ShopActionProp (Leave / Reroll counter-end props).
    shop_action_prop_instances: Vec<LitMeshInstance>,
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
    /// Per-bug wing instance slots.
    bug_wing_instances: Vec<LitMeshInstance>,
    /// Alpha-blended ghost-trail instances (body + wings share a slot).
    /// Indexed 0..MAX_BUG_GHOST_SLOTS. Rendered with `lit_mesh_blended_pipeline`.
    bug_ghost_body_instances: Vec<LitMeshInstance>,
    bug_ghost_wing_instances: Vec<LitMeshInstance>,
    /// Unit sphere mesh shared by every material-preview orb. The scene
    /// supplies the per-instance `MaterialParams` so a single mesh previews
    /// every `MaterialKind`.
    orb_mesh: LitMeshGpu,
    /// Per-orb instances for the material viewer scene. Bound with the 1×1
    /// default albedo and relief textures — materials that sample heightmaps
    /// render flat, previewing the shading model rather than any per-asset
    /// heightmap.
    orb_instances: Vec<LitMeshInstance>,
    /// Ofuda scroll model + pick_id (shop path-sign), one-frame-stale.
    last_ofuda_model: Option<(Mat4, u32)>,
    /// Info plaque model + pick_id (shop hover plaque), one-frame-stale.
    last_info_plaque_model: Option<(Mat4, u32)>,
    /// Leave action prop model + pick_id, one-frame-stale.
    last_leave_prop_model: Option<(Mat4, u32)>,
    /// Reroll action prop model + pick_id, one-frame-stale.
    last_reroll_prop_model: Option<(Mat4, u32)>,
    /// Sell tray model + pick_id, one-frame-stale.
    last_sell_tray_model: Option<(Mat4, u32)>,
    /// Single instance for the shop's curio cabinet.
    cabinet_instance: LitMeshInstance,
    /// Per-shrine instances (pick-blind scene). Indexed sequentially by
    /// `ShrineBatch` placement order; truncated at `MAX_SHRINE_SLOTS`.
    shrine_instances: Vec<LitMeshInstance>,
    /// Per-explicit-dish instances (shop scene). Indexed sequentially by
    /// `DishExplicit` placement order; grown on demand.
    aux_dish_instances: Vec<LitMeshInstance>,
    /// Per-ribbon world-space model matrices for `pick_shop_object`.
    last_ribbon_models: Vec<Mat4>,
    /// Total number of ribbon draw-slots populated this frame (across all
    /// `ZodiacBatch` cmds). Used by the shadow pass.
    last_ribbon_slot_count: usize,
    /// Per-batch ribbon slot counts: `last_ribbon_batch_slot_counts[batch_idx]`
    /// is how many draw-slots that batch consumed (2-3 per textured ribbon,
    /// 1 per untextured).
    last_ribbon_batch_slot_counts: Vec<usize>,
    /// Per-talisman world-space model matrices for `pick_shop_object`.
    last_talisman_models: Vec<Mat4>,
    /// Cabinet world AABB ((center, half_extents)) and screen rect for the
    /// current frame. Used so the shop scene can position the back-wall
    /// hover spotlight without re-deriving world coords.
    last_cabinet_world_aabb: Option<(glam::Vec3, glam::Vec3)>,
    /// World-space `(center, half_extents)` parallel with
    /// `last_aux_dish_rects`, used by `pick_shop_object` for AABB raycasts.
    last_aux_dish_aabbs: Vec<(glam::Vec3, glam::Vec3)>,

    // ── Skeuomorphic gameplay HUD meshes (phase 1 infrastructure) ──────
    plaque_mesh: LitMeshGpu,
    ofuda_mesh: LitMeshGpu,
    bone_tablet_mesh: LitMeshGpu,
    wood_tablet_mesh: LitMeshGpu,
    bowl_mesh: LitMeshGpu,
    mirror_mesh: LitMeshGpu,
    tally_stick_base_mesh: LitMeshGpu,
    tally_stick_tip_mesh: LitMeshGpu,
    plaque_instances: Vec<LitMeshInstance>,
    ofuda_instances: Vec<LitMeshInstance>,
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
    /// Per-falling-bone instances. Reuses `bone_tablet_mesh` like the cascade
    /// tokens, but each instance gets a full 3D model matrix (translation +
    /// euler tumble) so the bones look like real physical objects falling
    /// onto the play space during scoring.
    falling_bone_instances: Vec<LitMeshInstance>,
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
    /// Three reusable lit-mesh instances for the debug world-axes overlay
    /// (one per axis: 0 = X red, 1 = Y green, 2 = Z blue). Drawn through
    /// the shared `relic_box_mesh` unit cube; per-frame uniforms position
    /// and stretch each instance into a thin colored bar.
    debug_axes_instances: Vec<LitMeshInstance>,
    /// Per-yaku-tablet world-space model matrices for `pick_gameplay_object`.
    /// Parallel with `last_projected_yaku_tablet_rects`.
    last_yaku_tablet_models: Vec<Mat4>,
    /// Per-wood-tablet world-space model matrices for `pick_gameplay_object`.
    /// Index 0 = sort suit, 1 = sort rank, 2 = play hand.
    last_wood_tablet_models: Vec<Mat4>,
    /// Discard bowl world-space model matrix for `pick_gameplay_object`.
    last_bowl_model: Option<Mat4>,
    /// Bronze mirror world-space model matrix for `pick_gameplay_object`.
    last_mirror_model: Option<Mat4>,
    /// Per-frame catch-all of "what's this thing under the cursor" entries
    /// used by the debug "Object Hit Test" menu action. Each entry is a
    /// `(name, model, half_extents, center_offset_y)` tuple — the same
    /// local-space slab-test format the existing `pick_*_object` methods
    /// use, just with a human-readable name attached. Populated as the
    /// renderer walks the frame's draw cmds; consumed by
    /// `pick_debug_object`.
    last_debug_pickables: Vec<(String, Mat4, glam::Vec3, f32)>,
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
    #[allow(dead_code)] // Owns the GPU resource backing `shadow_map_view`.
    shadow_map_texture: wgpu::Texture,
    shadow_map_view: wgpu::TextureView,
    /// Bind-group layout for per-caster uniforms (group 0 of the shadow
    /// pipeline). Each `LitMeshInstance` and `HandTileGpu` owns one bind
    /// group built against this layout.
    shadow_caster_layout: wgpu::BindGroupLayout,
    /// Bind-group layout for the frame-shared shadow sampling group
    /// (group 2 of every 3D scene pipeline).
    #[allow(dead_code)] // Layout is consumed into bind groups at init; kept for resource lifetime.
    shadow_sample_layout: wgpu::BindGroupLayout,
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
pub const MAX_RELIC_SLOTS: usize = 16;
/// Maximum number of zodiac/talisman ribbon *draw slots* per frame (across all
/// `ZodiacBatch` cmds). Each textured ribbon uses up to 3 slots (top/mid/bot
/// caps), so 16 logical ribbons × 3 = 48. Truncated silently.
pub const MAX_RIBBON_SLOTS: usize = 48;
/// Maximum number of talisman tablets rendered per frame.
pub const MAX_TALISMAN_SLOTS: usize = 8;
/// Maximum number of physical coins rendered per frame (across all
/// `CoinBatch` cmds). The shop tooltip still shows the true gold count when
/// it exceeds this; the visual just caps the pile.
pub const MAX_COIN_SLOTS: usize = 64;
/// Maximum number of gold bars rendered per frame (across all `GoldBarBatch`
/// cmds). With big bars worth 100 and mini bars worth 25, 16 slots covers
/// absurdly high gold counts.
pub const MAX_BAR_SLOTS: usize = 16;
/// Maximum number of 3D bugs (insects near the lamp) rendered per frame.
pub const MAX_BUG_SLOTS: usize = 8;
/// Maximum number of bug ghost-trail instances per frame — each live bug
/// emits a chain of faded past-position copies for motion blur. Sized for
/// `MAX_BUG_SLOTS * BUG_TRAIL_SAMPLES` with headroom.
pub const MAX_BUG_GHOST_SLOTS: usize = 48;
/// Maximum number of material-preview orbs rendered per frame. Only the
/// material viewer debug scene uses these; 32 covers every `MaterialKind`
/// with room to grow.
pub const MAX_ORB_SLOTS: usize = 32;
/// Maximum number of explicit auxiliary dishes per frame (the shop uses 2:
/// the relic dish and the coin dish).
/// Maximum number of shrine instances per frame (pick-blind uses 3: Small,
/// Big, Boss). Truncated silently.
pub const MAX_SHRINE_SLOTS: usize = 4;
/// Maximum number of hanging ofuda per frame (gameplay uses 1).
pub const MAX_OFUDA_SLOTS: usize = 2;
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
/// Maximum number of physical falling-bone instances in flight at once.
/// Sized to comfortably hold a multi-step cascade's worth of bursts (each
/// scoring step spawns a small handful) without overflowing the pool.
pub const MAX_FALLING_BONE_SLOTS: usize = 192;
/// Maximum number of in-flight 3D extruded-glyph score popups. A single
/// cascade rarely fires more than 8-10 steps, so 32 is plenty for the
/// per-step popups plus the running-total readout that holds across the
/// final beat.
/// Score reel uses up to 2 × N_COLS slots (prev + current per spinning column)
/// plus popup labels. 48 gives headroom for reel overflow columns.
pub const MAX_EXTRUDED_GLYPH_SLOTS: usize = 80;

/// Pre-loaded background texture + bind group for the image pipeline.
struct BackgroundTextureGpu {
    #[allow(dead_code)]
    texture: wgpu::Texture,
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
        RelicRenderMaterial::Metal => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.78 + base_color[0] * 0.18,
                0.62 + base_color[1] * 0.16,
                0.22 + base_color[2] * 0.10,
                base_color[3],
            ],
            specular_strength: 0.52 + 0.24 * g,
            specular_power: 34.0,
        },
        RelicRenderMaterial::Plastic => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.74 + base_color[0] * 0.14,
                0.58 + base_color[1] * 0.12,
                0.24 + base_color[2] * 0.08,
                base_color[3],
            ],
            specular_strength: 0.44 + 0.18 * g,
            specular_power: 28.0,
        },
        RelicRenderMaterial::Glass => MaterialParams {
            kind: MaterialKind::Glass,
            base_color,
            specular_strength: 0.95 + 0.20 * g,
            specular_power: 96.0,
        },
        RelicRenderMaterial::Wax => MaterialParams {
            kind: MaterialKind::Wax,
            base_color,
            specular_strength: 0.08 + 0.10 * g,
            specular_power: 14.0,
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

fn make_showcase_tile_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    shadow_caster_layout: &wgpu::BindGroupLayout,
    primitives: &[TilePrimitiveGpu],
    sampler: &wgpu::Sampler,
    base_color_factor: [f32; 4],
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    tile: &Tile,
    tile_set: Option<&str>,
) -> ShowcaseTileGpu {
    let identity = Mat4::IDENTITY;
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
            cam_pos: [0.0; 3],
            _pad: 0.0,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    // Use `true` (hand-tile quality) so hand-strip tiles get the same
    // full-resolution decal as the old HandTileGpu path did.
    let rgba =
        rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set, true);
    let (decal_texture, decal_view) = upload_rgba_texture(
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
            _pad: 0.0,
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
        decal_texture,
    }
}

// ---------------------------------------------------------------------------
// WgpuRenderer impl
// ---------------------------------------------------------------------------

impl WgpuRenderer {
    pub fn new(window: Arc<Window>, hdr_enabled: bool) -> anyhow::Result<Self> {
        let t_total = Instant::now();
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window)?;

        let t0 = Instant::now();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| anyhow::anyhow!("adapter: {e:?}"))?;
        log::info!("wgpu adapter acquired in {:?}", t0.elapsed());

        let caps = surface.get_capabilities(&adapter);
        let format = if hdr_enabled {
            // Prefer Rgba16Float for HDR output; fall back to sRGB if unsupported.
            if caps.formats.contains(&wgpu::TextureFormat::Rgba16Float) {
                log::info!("HDR enabled — using Rgba16Float surface format");
                wgpu::TextureFormat::Rgba16Float
            } else {
                log::warn!("HDR requested but Rgba16Float not supported; falling back to sRGB");
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
        let (moon_albedo_texture, moon_albedo_view) =
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

        // ---- Shadow map resources (depth texture + sampler + layouts) ----
        // Built up here so the shared sampling layout can be plumbed into
        // both `tile_layout` and `lit_mesh_pl` below as group 2.
        const SHADOW_MAP_SIZE: u32 = 2048;
        let shadow_map_texture = device.create_texture(&wgpu::TextureDescriptor {
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
            shadow_map_texture.create_view(&wgpu::TextureViewDescriptor::default());
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

        // Flame pipeline — additive procedural fire on a quad. Reuses
        // quad_layout (only needs globals.time) and shares the unit-quad
        // vertex/instance buffers with quad_pipeline.
        let flame_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flame.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/flame.wgsl")).into(),
            ),
        });

        let flame_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flame-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &flame_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
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
                ..Default::default()
            },
            depth_stencil: Some(depth_ui.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
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
        let shooting_star_cascade_pipeline = vignette_pipeline(
            "shooting-star-cascade-pipeline",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/shooting_star_cascade.wgsl"
            )),
        );

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
                buffers: &[tile_vertex_layout.clone()],
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
        let lit_mesh_ssr_layout = create_lit_mesh_ssr_layout(&device);
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
                    let (albedo_texture, albedo_view) = match &prim.albedo_rgba {
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
                        albedo_texture,
                        albedo_view,
                        base_color_factor: prim.base_color_factor,
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
        let (lit_mesh_relief_default_tex, lit_mesh_relief_default_view) =
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
        let dish_mesh = LitMeshGpu::new(&device, &build_dish_mesh(), "relic-dish");
        let round_dish_mesh = LitMeshGpu::new(&device, &build_round_dish_mesh(), "round-dish");
        let sell_card_mesh = LitMeshGpu::new(&device, &build_tent_card_mesh(), "sell-card");
        let relic_box_mesh = LitMeshGpu::new(&device, &build_relic_mesh(), "relic-mesh");
        let pack_mesh = LitMeshGpu::new(&device, &build_pack_mesh(), "pack-mesh");
        let shop_action_prop_mesh =
            LitMeshGpu::new(&device, &build_shop_action_prop_mesh(), "shop-action-prop");
        let ribbon_mesh = LitMeshGpu::new(&device, &build_ribbon_mesh(), "ribbon");
        let coin_mesh = LitMeshGpu::new(&device, &build_coin_mesh(), "coin");
        let talisman_mesh = LitMeshGpu::new(&device, &build_talisman_mesh(), "talisman");
        let cabinet_mesh = LitMeshGpu::new(&device, &build_curio_cabinet_mesh(), "curio-cabinet");
        let book_mesh = LitMeshGpu::new(&device, &build_book_mesh(), "book");
        let shrine_mesh = LitMeshGpu::new(&device, &build_shrine_mesh(), "shrine");
        let lamp_body_mesh = LitMeshGpu::new(&device, &build_lamp_body_mesh(), "lamp-body");
        let lamp_bulb_mesh = LitMeshGpu::new(&device, &build_lamp_bulb_mesh(), "lamp-bulb");
        // Skeuomorphic gameplay HUD meshes (phase 1).
        let plaque_mesh = LitMeshGpu::new(&device, &build_plaque_mesh(), "plaque");
        let ofuda_mesh = LitMeshGpu::new(&device, &build_ofuda_mesh(), "ofuda");
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
        let (lit_mesh_white_tex, lit_mesh_white_view) = white_albedo(&device, &queue);

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
        let dish_instance = LitMeshInstance::new(
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
        let (lit_mesh_coin_height_tex, lit_mesh_coin_height_view) =
            load_coin_heightmap(&device, &queue);
        let mut coin_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_COIN_SLOTS);
        for _ in 0..MAX_COIN_SLOTS {
            coin_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_coin_height_view,
                &lit_mesh_coin_height_view,
                &tile_sampler,
            ));
        }
        // Gold bar instances — same Metal material as coins, reuses the coin
        // heightmap for a subtle engraved look on bar faces.
        let mut bar_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BAR_SLOTS);
        for _ in 0..MAX_BAR_SLOTS {
            bar_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_coin_height_view,
                &lit_mesh_coin_height_view,
                &tile_sampler,
            ));
        }
        // Single book instance for the journal bookend.
        let book_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );
        // Up to 2 shop action prop instances (Leave / Reroll counter-end props).
        let shop_action_prop_instances: Vec<LitMeshInstance> = (0..2)
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
            .collect();
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
        let mut bug_body_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
        let mut bug_wing_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
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
        }
        let mut bug_ghost_body_instances: Vec<LitMeshInstance> =
            Vec::with_capacity(MAX_BUG_GHOST_SLOTS);
        let mut bug_ghost_wing_instances: Vec<LitMeshInstance> =
            Vec::with_capacity(MAX_BUG_GHOST_SLOTS);
        for _ in 0..MAX_BUG_GHOST_SLOTS {
            bug_ghost_body_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
            bug_ghost_wing_instances.push(LitMeshInstance::new(
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
        let mut talisman_height_textures: Vec<wgpu::Texture> = Vec::new();
        let mut talisman_height_views: Vec<wgpu::TextureView> = Vec::new();
        for &(path, label) in &talisman_height_paths {
            let (tex, view) = load_metal_heightmap(&device, &queue, path, label);
            talisman_height_textures.push(tex);
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
        let cabinet_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );
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
        let aux_dish_instances: Vec<LitMeshInstance> = Vec::new();

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
        let plaque_instances: Vec<LitMeshInstance> = Vec::new();
        let ofuda_instances = make_pool(MAX_OFUDA_SLOTS);
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
        let falling_bone_instances = make_pool(MAX_FALLING_BONE_SLOTS);
        let extruded_glyph_instances = make_pool(MAX_EXTRUDED_GLYPH_SLOTS);
        let debug_axes_instances = make_pool(3);

        // Build the GPU profiler up-front while we still have a borrow of
        // device/queue (the struct literal below moves them).
        let gpu_profiler =
            crate::render::gpu_profiler::GpuProfiler::new(&device, &queue, timestamp_supported);

        log::info!("WgpuRenderer::new() total: {:?}", t_total.elapsed());

        Ok(Self {
            surface,
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
            flame_pipeline,
            starfield_pipeline,
            ember_drift_pipeline,
            golden_dust_pipeline,
            moonlit_water_pipeline,
            moon_albedo_texture,
            moon_albedo_bind_group,
            sunlit_water_pipeline,
            shooting_star_cascade_pipeline,
            tile_pipeline,
            tile_outline_pipeline,
            tile_glow_pipeline,
            globals_buffer,
            globals_bind_group,
            tile_material_layout,
            point_lights_buffer,
            tile_occluders_buffer,
            point_lights_bind_group,
            tile_sampler,
            tile_primitives,
            tile_base_color_factor,
            tile_set: Some("original".to_string()),
            hand_tiles: Vec::new(),
            showcase_tiles: Vec::new(),
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
            relic_slot_texture: vec![None; MAX_RELIC_SLOTS],
            pack_instances,
            pack_slot_texture: vec![None; 4],
            ribbon_mesh,
            talisman_mesh,
            coin_mesh,
            cabinet_mesh,
            shrine_mesh,
            ribbon_instances,
            ribbon_slot_zodiac,
            ribbon_zodiac_tex,
            talisman_instances,
            coin_instances,
            bar_instances,
            book_mesh,
            book_instance,
            last_book_model: None,
            shop_action_prop_instances,
            sell_tray_instance,
            sell_card_instance,
            sell_card_decal_ready: false,
            last_sell_card_model: None,
            lamp_body_mesh,
            lamp_bulb_mesh,
            lamp_body_instance,
            lamp_bulb_instance,
            bug_body_mesh,
            bug_wing_mesh,
            bug_body_instances,
            bug_wing_instances,
            bug_ghost_body_instances,
            bug_ghost_wing_instances,
            orb_mesh,
            orb_instances,
            last_ofuda_model: None,
            last_info_plaque_model: None,
            last_leave_prop_model: None,
            last_reroll_prop_model: None,
            last_sell_tray_model: None,
            cabinet_instance,
            shrine_instances,
            aux_dish_instances,
            last_ribbon_models: Vec::new(),
            last_ribbon_slot_count: 0,
            last_ribbon_batch_slot_counts: Vec::new(),
            last_talisman_models: Vec::new(),
            last_cabinet_world_aabb: None,
            last_aux_dish_aabbs: Vec::new(),
            plaque_mesh,
            ofuda_mesh,
            bone_tablet_mesh,
            wood_tablet_mesh,
            bowl_mesh,
            mirror_mesh,
            tally_stick_base_mesh,
            tally_stick_tip_mesh,
            plaque_instances,
            ofuda_instances,
            yaku_tablet_instances,
            wood_tablet_instances,
            bowl_instances,
            mirror_instances,
            tally_stick_instances,
            wall_tile_instances,
            cascade_token_instances,
            falling_bone_instances,
            extruded_glyph_instances,
            glyph_cpu_cache: crate::render::glyph_mesh::GlyphMeshCache::new(),
            extruded_glyph_meshes: HashMap::new(),
            debug_axes_instances,
            last_yaku_tablet_models: Vec::new(),
            last_wood_tablet_models: Vec::new(),
            last_bowl_model: None,
            last_mirror_model: None,
            last_debug_pickables: Vec::new(),
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
            lit_mesh_white_tex,
            lit_mesh_white_view,
            lit_mesh_relief_default_tex,
            lit_mesh_relief_default_view,
            lit_mesh_coin_height_tex,
            lit_mesh_coin_height_view,
            talisman_height_textures,
            talisman_height_views,
            talisman_slot_kind,
            candle_wax_mesh,
            candle_wick_mesh,
            table_mesh,
            dish_mesh,
            round_dish_mesh,
            sell_card_mesh,
            relic_box_mesh,
            pack_mesh,
            shop_action_prop_mesh,
            relic_meshes: HashMap::new(),
            candle_instances,
            table_instance,
            dish_instance,
            relic_instances,
            shadow_map_texture,
            shadow_map_view,
            shadow_caster_layout,
            shadow_sample_layout,
            shadow_globals_buffer,
            shadow_sample_bind_group,
            shadow_pipeline,
            gpu_profiler,
        })
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
                    let (tex, view) = upload_rgba_texture(
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
                    self.background_textures.insert(
                        img.id,
                        BackgroundTextureGpu {
                            texture: tex,
                            bind_group,
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

    /// Cast a ray from the camera through the cursor (in physical pixels,
    /// matching the renderer's surface size) and return the index of the
    /// closest hand tile whose OBB the ray hits, if any.
    ///
    /// The intersection is done in each tile's *local* mesh space: the world
    /// ray is transformed by the inverse model matrix and tested against the
    /// normalized mesh's local AABB (centered at origin, half-extents
    /// `LOCAL_*_EXTENT / 2`). Model matrices include
    /// [`crate::render::table_transform::tile_mesh_local_to_world`].
    ///
    /// Uses the previous frame's snapshot, so this is consistent with what
    /// the user actually saw last frame (one-frame-stale, like the projected
    /// hand rects).
    pub fn pick_hand_tile(&self, cursor_x: f32, cursor_y: f32) -> Option<usize> {
        let cam = self.last_pick_camera.as_ref()?;
        if self.last_pick_models.is_empty() {
            return None;
        }

        // Cursor → NDC. wgpu uses z ∈ [0, 1] (matches Mat4::perspective_rh).
        let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;

        // Unproject near and far points to world space.
        let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
        let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }

        // Local AABB of the normalized tile mesh.
        let hx = LOCAL_X_EXTENT * 0.5;
        let hy = LOCAL_Y_EXTENT * 0.5;
        let hz = LOCAL_Z_EXTENT * 0.5;

        let mut best: Option<(usize, f32)> = None;
        for &(i, model) in &self.last_pick_models {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);

            // Slab test against [-h, h] on each axis.
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            let bounds = [(lo.x, ld.x, hx), (lo.y, ld.y, hy), (lo.z, ld.z, hz)];
            let mut hit = true;
            for (o, d, h) in bounds {
                if d.abs() < 1e-8 {
                    if o < -h || o > h {
                        hit = false;
                        break;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (-h - o) * inv_d;
                    let mut t2 = (h - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        hit = false;
                        break;
                    }
                }
            }
            if !hit {
                continue;
            }
            // Ignore boxes that are entirely behind the camera.
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 {
                continue;
            }
            match best {
                Some((_, bt)) if t_enter >= bt => {}
                _ => best = Some((i, t_enter)),
            }
        }
        best.map(|(i, _)| i)
    }

    /// Cast a ray from the camera through the cursor and return the closest
    /// shop object hit. Uses the same one-frame-stale snapshot pattern as
    /// `pick_hand_tile`.
    pub fn pick_shop_object(&self, cursor_x: f32, cursor_y: f32) -> Option<ShopHit> {
        let cam = self.last_pick_camera.as_ref()?;
        if self.last_relic_models.is_empty()
            && self.last_ribbon_models.is_empty()
            && self.last_talisman_models.is_empty()
            && self.proj.aux_dish_rects.is_empty()
        {
            return None;
        }

        let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
        let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
        let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }

        // Slab test against the unit cube [-0.5, 0.5]^3 in local space, after
        // transforming the world ray by the inverse model matrix. Used for
        // both relics (relic_box_mesh) and ribbons whose mesh local bounds
        // sit in [-0.5,0.5] x [-1,0] x ~0; we test the unit cube and accept
        // false positives near the empty top of the ribbon (small enough that
        // it doesn't matter for hover).
        let slab_test = |model: glam::Mat4, hx: f32, hy: f32, hz: f32, oy: f32| -> Option<f32> {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);
            let bounds = [
                (lo.x, ld.x, -hx, hx),
                (lo.y, ld.y, -hy + oy, hy + oy),
                (lo.z, ld.z, -hz, hz),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        return None;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        return None;
                    }
                }
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 { None } else { Some(t_enter) }
        };

        let mut best: Option<(ShopHit, f32)> = None;
        let mut consider = |hit: ShopHit, t: f32| match best {
            Some((_, bt)) if t >= bt => {}
            _ => best = Some((hit, t)),
        };

        // Relic cuboids — local bounds [-0.5, 0.5]^3.
        for (i, model) in self.last_relic_models.iter().enumerate() {
            if let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0) {
                consider(ShopHit::Relic(i), t);
            }
        }
        // Ribbons — local bounds x ∈ [-0.5,0.5], y ∈ [-1, 0], z ∈ [-0.05, 0.05].
        // Express as half-extents (0.5, 0.5, 0.5) centered at y=-0.5 via offset.
        for (i, model) in self.last_ribbon_models.iter().enumerate() {
            if let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, -0.5) {
                consider(ShopHit::Ribbon(i), t);
            }
        }
        // Talismans — local AABB from TALISMAN_LOCAL_HALF, centered at origin.
        for (i, model) in self.last_talisman_models.iter().enumerate() {
            if let Some(t) = slab_test(
                *model,
                TALISMAN_LOCAL_HALF[0],
                TALISMAN_LOCAL_HALF[1],
                TALISMAN_LOCAL_HALF[2],
                0.0,
            ) {
                consider(ShopHit::Talisman(i), t);
            }
        }
        // Book — local-space slab test (handles rotation correctly).
        // The book mesh spans x ∈ [−0.58, 0.5] (spine bulge), y/z ∈ [−0.5, 0.5].
        // Use half-extents (0.54, 0.5, 0.5) centered at x = −0.04 to cover the
        // full convex hull of the mesh.
        if let Some((model, pid)) = self.last_book_model {
            if let Some(t) = slab_test(model, 0.54, 0.5, 0.5, 0.0) {
                consider(ShopHit::Dish(pid), t);
            }
        }
        // Ofuda path-sign scroll.
        if let Some((model, pid)) = self.last_ofuda_model {
            if let Some(t) = slab_test(model, 0.5, 0.5, 0.1, 0.0) {
                consider(ShopHit::Dish(pid), t);
            }
        }
        // Shop action props (Leave / Reroll counter-end).
        if let Some((model, pid)) = self.last_reroll_prop_model {
            if let Some(t) = slab_test(model, 0.5, 0.5, 0.5, 0.0) {
                consider(ShopHit::Dish(pid), t);
            }
        }
        if let Some((model, pid)) = self.last_leave_prop_model {
            if let Some(t) = slab_test(model, 0.5, 0.5, 0.5, 0.0) {
                consider(ShopHit::Dish(pid), t);
            }
        }
        // Sell tray.
        if let Some((model, pid)) = self.last_sell_tray_model {
            if let Some(t) = slab_test(model, 0.5, 0.5, 0.5, 0.0) {
                consider(ShopHit::Dish(pid), t);
            }
        }
        // Auxiliary dishes (world-space AABB picks).
        for (i, (id, _rect)) in self.proj.aux_dish_rects.iter().enumerate() {
            let Some(pid) = id else { continue };
            let Some((center, half)) = self.last_aux_dish_aabbs.get(i) else {
                continue;
            };
            // World-space AABB slab test.
            let bounds = [
                (
                    world_origin.x,
                    world_dir.x,
                    center.x - half.x,
                    center.x + half.x,
                ),
                (
                    world_origin.y,
                    world_dir.y,
                    center.y - half.y,
                    center.y + half.y,
                ),
                (
                    world_origin.z,
                    world_dir.z,
                    center.z - half.z,
                    center.z + half.z,
                ),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            let mut hit = true;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        hit = false;
                        break;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        hit = false;
                        break;
                    }
                }
            }
            if !hit {
                continue;
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 {
                continue;
            }
            consider(ShopHit::Dish(*pid), t_enter);
        }

        // Pack boxes — 2D projected rect hit test (packs are few, so a
        // simple screen-space check is sufficient).
        for (rect, pick_id) in &self.proj.pack_rects {
            let Some(pid) = pick_id else { continue };
            let [rx, ry, rw, rh] = *rect;
            if cursor_x >= rx && cursor_x <= rx + rw && cursor_y >= ry && cursor_y <= ry + rh {
                // Use a small t value so nearby 3D picks can still win.
                consider(ShopHit::TilePack(*pid), 0.5);
            }
        }

        best.map(|(h, _)| h)
    }

    /// Cast a ray from the camera through the cursor and return the closest
    /// gameplay-scene object hit (yaku tablet, wood action tablet, or
    /// discard bowl). One-frame-stale snapshot pattern, mirroring
    /// `pick_hand_tile` and `pick_shop_object`. The per-class local AABBs
    /// are precomputed mesh constants — there is no per-frame screen-space
    /// projection in the hit-test path.
    pub fn pick_gameplay_object(&self, cursor_x: f32, cursor_y: f32) -> Option<GameplayPick> {
        let cam = self.last_pick_camera.as_ref()?;
        if self.last_yaku_tablet_models.is_empty()
            && self.last_wood_tablet_models.is_empty()
            && self.last_bowl_model.is_none()
            && self.last_mirror_model.is_none()
        {
            return None;
        }
        let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
        let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
        let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }
        // Local-space slab test against an AABB centered at `(0, oy, 0)` with
        // half-extents `(hx, hy, hz)`. Returns the entry distance along the
        // world ray when the ray hits the box.
        let slab_test = |model: glam::Mat4, hx: f32, hy: f32, hz: f32, oy: f32| -> Option<f32> {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);
            let bounds = [
                (lo.x, ld.x, -hx, hx),
                (lo.y, ld.y, -hy + oy, hy + oy),
                (lo.z, ld.z, -hz, hz),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        return None;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        return None;
                    }
                }
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 { None } else { Some(t_enter) }
        };

        let mut best: Option<(GameplayPick, f32)> = None;
        let mut consider = |hit: GameplayPick, t: f32| match best {
            Some((_, bt)) if t >= bt => {}
            _ => best = Some((hit, t)),
        };

        // Yaku tablets — unit cube `[-0.5, 0.5]^3` (push_box convention).
        for (i, model) in self.last_yaku_tablet_models.iter().enumerate() {
            if let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0) {
                consider(GameplayPick::YakuTablet(i), t);
            }
        }
        // Wood action tablets — same unit cube as the yaku tablets.
        for (i, model) in self.last_wood_tablet_models.iter().enumerate() {
            if let Some(t) = slab_test(*model, 0.5, 0.5, 0.5, 0.0) {
                consider(GameplayPick::WoodTablet(i), t);
            }
        }
        // Discard bowl — tighter local AABB from the bowl mesh constants.
        if let Some(model) = self.last_bowl_model.as_ref() {
            if let Some(t) = slab_test(
                *model,
                BOWL_LOCAL_HALF[0],
                BOWL_LOCAL_HALF[1],
                BOWL_LOCAL_HALF[2],
                BOWL_LOCAL_CENTER_Y,
            ) {
                consider(GameplayPick::DiscardBowl, t);
            }
        }
        // Bronze mirror — flat disc local AABB from the mirror mesh constants.
        if let Some(model) = self.last_mirror_model.as_ref() {
            if let Some(t) = slab_test(
                *model,
                MIRROR_LOCAL_HALF[0],
                MIRROR_LOCAL_HALF[1],
                MIRROR_LOCAL_HALF[2],
                MIRROR_LOCAL_CENTER_Y,
            ) {
                consider(GameplayPick::BronzeMirror, t);
            }
        }

        best.map(|(h, _)| h)
    }

    /// Debug "what is the cursor over?" picker. Walks
    /// `last_debug_pickables` (populated as the renderer processes the
    /// frame's draw cmds) and returns the closest hit's name. Hand tiles
    /// are checked separately because they have their own pick path.
    /// Returns `None` if nothing was hit.
    pub fn pick_debug_object(&self, cursor_x: f32, cursor_y: f32) -> Option<String> {
        // Hand tiles first — they have their own dedicated picker that
        // already handles per-tile OBBs.
        if let Some(idx) = self.pick_hand_tile(cursor_x, cursor_y) {
            return Some(format!("gameplay.hand.tile[{}]", idx));
        }

        let cam = self.last_pick_camera.as_ref()?;
        if self.last_debug_pickables.is_empty() {
            return None;
        }
        let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
        let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
        let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }

        let slab_test = |model: glam::Mat4, half: glam::Vec3, oy: f32| -> Option<f32> {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);
            let bounds = [
                (lo.x, ld.x, -half.x, half.x),
                (lo.y, ld.y, -half.y + oy, half.y + oy),
                (lo.z, ld.z, -half.z, half.z),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        return None;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        return None;
                    }
                }
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 { None } else { Some(t_enter) }
        };

        let mut best: Option<(&str, f32)> = None;
        for (name, model, half, oy) in &self.last_debug_pickables {
            if let Some(t) = slab_test(*model, *half, *oy) {
                match best {
                    Some((_, bt)) if t > bt => {}
                    _ => best = Some((name.as_str(), t)),
                }
            }
        }
        best.map(|(n, _)| n.to_string())
    }

    /// Set (or clear) the arrange-mode model-matrix override. Called each frame
    /// from `App` when arrange mode has a selected object. Pass `None` to clear.
    pub fn set_arrange_override(&mut self, ov: Option<DebugArrangeOverride>) {
        self.debug_arrange_override = ov;
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
            Mat4::from_cols(nx.extend(0.0), ny.extend(0.0), nz.extend(0.0), t.extend(1.0))
        } else {
            model
        };
        // Translation delta (only applies while a delta is staged for this name).
        if let Some(ov) = staged {
            let dt = glam::Vec3::new(ov.delta_px, -ov.delta_py, ov.delta_lift);
            let t = model.w_axis.truncate() + dt;
            model = Mat4::from_cols(
                model.x_axis,
                model.y_axis,
                model.z_axis,
                t.extend(1.0),
            );
        }
        model
    }

    /// Returns the smoothed hover value for `anim_id`, advancing the
    /// exponential-ease envelope toward `target` by `frame_dt`.
    ///
    /// If `anim_id == 0`, returns `target` directly (no per-frame state).
    /// Rate ≈ 14 → ~70 ms time constant, matching the rest of the HUD.
    ///
    /// Note: cannot be called inside `draw_frame` because that function holds
    /// an `&self.scene_color_view` borrow for its entire scope. In those call
    /// sites, inline the body as `self.obj3d_hover_state.entry(id).or_insert(0.0)`.
    #[allow(dead_code)]
    fn ease_hover(&mut self, anim_id: u64, target: f32) -> f32 {
        if anim_id == 0 {
            return target;
        }
        let k = 1.0 - (-14.0 * self.frame_dt).exp();
        let entry = self.obj3d_hover_state.entry(anim_id).or_insert(0.0);
        *entry += (target - *entry) * k;
        *entry
    }

    /// Like [`Self::pick_debug_object`] but also returns the world-space model
    /// matrix of the closest hit. Used by arrange mode to seed the initial
    /// World-space translation of the pickable registered under `name` in
    /// the most recent frame. `None` if the name isn't currently pickable.
    pub fn debug_object_origin(&self, name: &str) -> Option<glam::Vec3> {
        self.last_debug_pickables
            .iter()
            .find(|(n, _, _, _)| n == name)
            .map(|(_, m, _, _)| m.transform_point3(glam::Vec3::ZERO))
    }

    /// Raycast the cursor ray against registered pickables and return the
    /// world-space hit point of the nearest intersection. Used by arrange
    /// mode's click-to-move so teleport targets land on actual geometry.
    pub fn pick_debug_world_point(&self, cursor_x: f32, cursor_y: f32) -> Option<glam::Vec3> {
        let cam = self.last_pick_camera.as_ref()?;
        if self.last_debug_pickables.is_empty() {
            return None;
        }
        let ndc_x = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ndc_y = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
        let near_clip = glam::Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far_clip = glam::Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }

        let slab_test = |model: glam::Mat4, half: glam::Vec3, oy: f32| -> Option<f32> {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);
            let bounds = [
                (lo.x, ld.x, -half.x, half.x),
                (lo.y, ld.y, -half.y + oy, half.y + oy),
                (lo.z, ld.z, -half.z, half.z),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        return None;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        return None;
                    }
                }
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 { None } else { Some(t_enter) }
        };

        let mut best_t: Option<f32> = None;
        for (_n, model, half, oy) in &self.last_debug_pickables {
            if let Some(t) = slab_test(*model, *half, *oy) {
                match best_t {
                    Some(bt) if t > bt => {}
                    _ => best_t = Some(t),
                }
            }
        }
        best_t.map(|t| world_origin + world_dir * t)
    }

    /// position and rotation from the object's current transform.
    /// Hand-tile hits return `None` for the model matrix (they don't have a
    /// single rigid placement matrix available here).
    pub fn pick_debug_object_with_model(
        &self,
        cursor_x: f32,
        cursor_y: f32,
    ) -> Option<(String, Option<glam::Mat4>)> {
        // Hand tiles: clicking any tile selects the whole strip as
        // `gameplay.hand.strip` so arrange mode can move/rotate all tiles
        // together as a group.
        if self.pick_hand_tile(cursor_x, cursor_y).is_some() {
            let target = "gameplay.hand.strip";
            let model = self
                .last_debug_pickables
                .iter()
                .find(|(n, _, _, _)| n == target)
                .map(|(_, m, _, _)| *m);
            return Some((target.to_string(), model));
        }

        let cam = self.last_pick_camera.as_ref()?;
        if self.last_debug_pickables.is_empty() {
            return None;
        }
        let nx = (cursor_x / cam.viewport_w) * 2.0 - 1.0;
        let ny = 1.0 - (cursor_y / cam.viewport_h) * 2.0;
        let near_clip = glam::Vec4::new(nx, ny, 0.0, 1.0);
        let far_clip = glam::Vec4::new(nx, ny, 1.0, 1.0);
        let near_w = cam.inv_view_proj * near_clip;
        let far_w = cam.inv_view_proj * far_clip;
        if near_w.w.abs() < 1e-6 || far_w.w.abs() < 1e-6 {
            return None;
        }
        let near = near_w.truncate() / near_w.w;
        let far = far_w.truncate() / far_w.w;
        let world_origin = near;
        let world_dir = (far - near).normalize_or_zero();
        if world_dir.length_squared() < 1e-6 {
            return None;
        }

        let slab_test = |model: glam::Mat4, half: glam::Vec3, oy: f32| -> Option<f32> {
            let inv = model.inverse();
            let lo = inv.transform_point3(world_origin);
            let ld = inv.transform_vector3(world_dir);
            let bounds = [
                (lo.x, ld.x, -half.x, half.x),
                (lo.y, ld.y, -half.y + oy, half.y + oy),
                (lo.z, ld.z, -half.z, half.z),
            ];
            let mut t_min = f32::NEG_INFINITY;
            let mut t_max = f32::INFINITY;
            for (o, d, lo_b, hi_b) in bounds {
                if d.abs() < 1e-8 {
                    if o < lo_b || o > hi_b {
                        return None;
                    }
                } else {
                    let inv_d = 1.0 / d;
                    let mut t1 = (lo_b - o) * inv_d;
                    let mut t2 = (hi_b - o) * inv_d;
                    if t1 > t2 {
                        std::mem::swap(&mut t1, &mut t2);
                    }
                    if t1 > t_min {
                        t_min = t1;
                    }
                    if t2 < t_max {
                        t_max = t2;
                    }
                    if t_min > t_max {
                        return None;
                    }
                }
            }
            let t_enter = if t_min >= 0.0 { t_min } else { t_max };
            if t_enter < 0.0 { None } else { Some(t_enter) }
        };

        let mut best: Option<(&str, f32, glam::Mat4)> = None;
        for (name, model, half, oy) in &self.last_debug_pickables {
            if let Some(t) = slab_test(*model, *half, *oy) {
                match best {
                    Some((_, bt, _)) if t > bt => {}
                    _ => best = Some((name.as_str(), t, *model)),
                }
            }
        }
        best.map(|(n, _, m)| (n.to_string(), Some(m)))
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);

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
    }

    /// Render one frame.
    ///
    /// `frame.cmds` is walked in order — earlier cmds render under later ones.
    /// Contiguous runs of `DrawCmd::Quad` are batched into a single instanced
    /// draw, which is invisible to scenes and preserves ordering.
    pub fn render(
        &mut self,
        frame: &UiFrame,
        smoke_intensity: crate::persistence::SmokeIntensity,
        smoke_detail: crate::persistence::SmokeDetail,
        smoke_sim_quality: crate::persistence::SmokeSimQuality,
        effects_quality: crate::persistence::EffectsQuality,
        tile_preset: crate::persistence::TilePreset,
        tile_material: crate::persistence::TileMaterial,
        draw_settle_speed: f32,
        sort_settle_speed: f32,
        gamma: f32,
        shadows_enabled: bool,
        ssr_enabled: bool,
    ) -> anyhow::Result<()> {
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

        let surface_frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Validation => {
                return Ok(());
            }
        };
        let view = surface_frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let scene_view = &self.scene_color_view;
        let bloom_active = frame
            .cmds
            .iter()
            .any(|cmd| matches!(cmd, DrawCmd::MoonlitWater));

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

        // ── Flame screen anchors ────────────────────────────────────────
        // The flame is a 2D additive quad in screen-pixel space, but it
        // needs to sit on top of a 3D candle wick whose screen position
        // depends on the gameplay-camera projection. Walk the cmd list,
        // find the CandleBatch, project each candle's wick tip with the
        // same view_proj we just built, and produce per-candle flame
        // rects (x, y, w, h) sized to match the candle's projected
        // pixel height. The Flame batching loop below consumes these in
        // order, overriding whatever the scene chose.
        //
        // We size the flame as a fraction of the *projected* candle
        // height so far candles get a smaller flame than near ones — the
        // perspective foreshortening is non-trivial because the four
        // votives sit at noticeably different depths on the table.
        let flame_anchors: Vec<([f32; 4], [f32; 2])> = {
            let mut out: Vec<([f32; 4], [f32; 2])> = Vec::new();
            for cmd in frame.cmds.iter() {
                if let DrawCmd::CandleBatch(placements) = cmd {
                    for p in placements.iter() {
                        let base_world = pixel_to_world(w, h, p.world_pos[0], p.world_pos[1], 0.0);
                        let tip_world = pixel_to_world(
                            w,
                            h,
                            p.world_pos[0],
                            p.world_pos[1],
                            crate::render::candle_mesh::WICK_TIP_Y * p.scale * p.height_scale,
                        );
                        let (_bsx, bsy) = project_to_screen(base_world);
                        let (tsx, tsy) = project_to_screen(tip_world);
                        // Projected pixel height of the candle from base
                        // to wick tip — used to scale the flame so it
                        // matches the candle's perspective foreshortening.
                        let candle_pix_h = (bsy - tsy).abs().max(1.0);
                        // Flame proportions relative to the candle's
                        // total projected height. These constants reproduce
                        // the original ~46×28 flame on a ~150-tall candle
                        // and scale gracefully with depth.
                        let flame_h = candle_pix_h * 0.42;
                        let flame_w = candle_pix_h * 0.26;
                        // Anchor: flame *base* sits at the wick tip. The
                        // shader maps `corner.y=1` to the bottom of the
                        // rect, so base_y = rect.y + rect.w; solve for
                        // rect.y = tip_sy - flame_h. Center horizontally
                        // around the projected wick.
                        let rect_x = tsx - flame_w * 0.5;
                        let rect_y = tsy - flame_h;

                        // Per-flame wind sample. Walk the active scene
                        // wind impulses, weight each by a soft falloff
                        // around its world-space radius, and project
                        // their world velocities through the same view
                        // we just used to anchor the flame. The result
                        // is a screen-space delta that the flame shader
                        // reads as `color.rg` and turns into a lateral
                        // bend + extra flicker. Normalised by the flame
                        // quad's own pixel size so distant candles bend
                        // by the same *visual* amount as near ones.
                        let mut wind_px = (0.0_f32, 0.0_f32);
                        for g in frame.wind_gusts.iter() {
                            let g_world =
                                pixel_to_world(w, h, g.center_px.0, g.center_px.1, g.lift);
                            let dist = (g_world - tip_world).length();
                            let r = (g.radius * 3.0).max(1.0);
                            let falloff = (1.0 - (dist / r).clamp(0.0, 1.0)).powf(1.5);
                            if falloff <= 0.0 {
                                continue;
                            }
                            // Project a small step along the gust
                            // velocity to get its screen-space direction
                            // at the candle's depth.
                            let v = glam::Vec3::new(g.velocity[0], g.velocity[1], g.velocity[2]);
                            // Step the velocity ~0.12s forward so even
                            // moderate gusts produce a clearly visible
                            // screen-space delta after the per-flame
                            // normalisation below.
                            let (psx, psy) = project_to_screen(tip_world + v * 0.12);
                            wind_px.0 += (psx - tsx) * falloff;
                            wind_px.1 += (psy - tsy) * falloff;
                        }
                        // Convert from absolute screen-pixel delta into
                        // flame-relative units (≈ multiples of the flame
                        // half-width). 1.0 ≈ "tip pulled to the edge of
                        // the quad" — the shader clamps at 1.5.
                        let inv_w = 1.0 / flame_w.max(1.0);
                        let inv_h = 1.0 / flame_h.max(1.0);
                        let wind_norm = [
                            (wind_px.0 * inv_w * 1.1).clamp(-1.5, 1.5),
                            (wind_px.1 * inv_h * 1.1).clamp(-1.5, 1.5),
                        ];

                        out.push(([rect_x, rect_y, flame_w, flame_h], wind_norm));
                    }
                    break; // assume only one candle batch per frame
                }
            }
            out
        };
        let mut next_flame_anchor: usize = 0;

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
                        if speed > 0.5 {
                            if let Some(ref mut fluid) = self.fluid {
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
                                _pad: 0.0,
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
                            _pad: 0.0,
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
            #[allow(dead_code)]
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
            let tw = (lbl.rect[2].max(1.0).min(16384.0) as u32).max(1);
            let th = (lbl.rect[3].max(1.0).min(16384.0) as u32).max(1);
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
                lbl.font_px,
                align,
                lbl.scroll_offset,
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
        struct ImageDraw {
            inst_buf: wgpu::Buffer,
            bind_group: wgpu::BindGroup,
        }

        enum RenderOp {
            Background(BackgroundId),
            Starfield,
            EmberDrift,
            GoldenDust,
            MoonlitWater,
            SunlitWater,
            ShootingStarCascade,
            Table,
            Dish,
            RelicBatch(usize),         // index into `relic_batches`
            RelicShowcaseBatch(usize), // index into `relic_showcase_batches`
            PackBatch(usize),          // index into `pack_batches`
            CandleBatch(usize),        // index into `candle_batches`
            DishExplicit(usize),       // index into `aux_dish_cmds`
            CurioCabinet, // single instance — only the most-recent CurioCabinet cmd is drawn
            ShrineBatch(usize), // index into `shrine_batches`
            ZodiacBatch(usize), // index into `ribbon_batches`
            TalismanBatch(usize), // index into `talisman_batches`
            CoinBatch(usize), // index into `coin_batches`
            GoldBarBatch(usize), // index into `bar_batches`
            Book,         // single book instance
            QuadBatch { buf_idx: usize, count: u32 },
            FlameBatch { buf_idx: usize, count: u32 },
            TextDraw(usize),
            RelicIconDraw(usize),
            FluidSmoke,
            // Skeuomorphic gameplay HUD (phase 1).
            Plaque(usize),                              // index into `plaque_cmds`
            Ofuda(usize),                               // index into `ofuda_cmds`
            YakuTabletBatch(usize),                     // index into `yaku_tablet_batches`
            WoodTabletBatch(usize),                     // index into `wood_tablet_batches`
            Bowl(usize),                                // index into `bowl_cmds`
            Mirror(usize),                              // index into `mirror_cmds`
            TallyFan(usize),                            // index into `tally_fan_cmds`
            WallStack(usize),                           // index into `wall_stack_cmds`
            CascadeTokenBatch(usize),                   // index into `cascade_token_batches`
            FallingBoneBatch(usize),                    // index into `falling_bone_batches`
            ExtrudedGlyphBatch(usize),                  // index into `extruded_glyph_batches`
            ShowcaseTileBatch(usize),                   // index into `showcase_tile_batches`
            Object3dBatch { start: usize, end: usize }, // range into `object3d_draw_list`
        }

        let mut quad_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut flame_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut text_draws: Vec<TextDraw> = Vec::new();
        let mut image_draws: Vec<ImageDraw> = Vec::new();
        let mut candle_batches: Vec<&[CandlePlacement]> = Vec::new();
        let mut relic_batches: Vec<&[RelicPlacement]> = Vec::new();
        let mut relic_showcase_batches: Vec<&[RelicShowcasePlacement]> = Vec::new();
        let mut pack_batches: Vec<&[PackPlacement]> = Vec::new();
        let mut ribbon_batches: Vec<&[ZodiacRibbonPlacement]> = Vec::new();
        let mut talisman_batches: Vec<&[TalismanPlacement]> = Vec::new();
        let mut coin_batches: Vec<&[CoinPlacement]> = Vec::new();
        let mut bar_batches: Vec<&[GoldBarPlacement]> = Vec::new();
        let mut book_cmd: Option<&BookPlacement> = None;
        let mut aux_dish_cmds: Vec<&DishExplicit> = Vec::new();
        let mut cabinet_cmds: Vec<&CurioCabinetPlacement> = Vec::new();
        let mut shrine_batches: Vec<&[ShrinePlacement]> = Vec::new();
        // Skeuomorphic gameplay HUD cmd buffers (phase 1).
        let mut plaque_cmds: Vec<&PlaquePlacement> = Vec::new();
        let mut ofuda_cmds: Vec<&OfudaPlacement> = Vec::new();
        let mut yaku_tablet_batches: Vec<&[YakuTabletPlacement]> = Vec::new();
        let mut wood_tablet_batches: Vec<&[WoodTabletPlacement]> = Vec::new();
        let mut bowl_cmds: Vec<&BowlPlacement> = Vec::new();
        let mut mirror_cmds: Vec<&MirrorPlacement> = Vec::new();
        let mut tally_fan_cmds: Vec<&TallyFanPlacement> = Vec::new();
        let mut wall_stack_cmds: Vec<&WallStackPlacement> = Vec::new();
        let mut cascade_token_batches: Vec<&[CascadeTokenPlacement]> = Vec::new();
        let mut falling_bone_batches: Vec<&[FallingBonePlacement]> = Vec::new();
        let mut extruded_glyph_batches: Vec<&[ExtrudedGlyphPlacement]> = Vec::new();
        let mut showcase_tile_batches: Vec<&[ShowcaseTilePlacement]> = Vec::new();
        let mut object3d_cmds: Vec<&[crate::render::draw_cmd::Object3d]> = Vec::new();
        // Flat draw list built during the Object3d pre-pass: (kind_id, slot_i)
        // kind_id: 0=Plaque, 1=Ofuda, 2=YakuTablet, 3=WoodTablet
        let mut object3d_draw_list: Vec<(u8, usize)> = Vec::new();
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
                DrawCmd::CandleBatch(placements) => {
                    let idx = candle_batches.len();
                    candle_batches.push(placements.as_slice());
                    ops.push(RenderOp::CandleBatch(idx));
                    i += 1;
                }
                DrawCmd::Dish => {
                    ops.push(RenderOp::Dish);
                    i += 1;
                }
                DrawCmd::RelicBatch(placements) => {
                    let idx = relic_batches.len();
                    relic_batches.push(placements.as_slice());
                    ops.push(RenderOp::RelicBatch(idx));
                    i += 1;
                }
                DrawCmd::RelicShowcaseBatch(placements) => {
                    let idx = relic_showcase_batches.len();
                    relic_showcase_batches.push(placements.as_slice());
                    ops.push(RenderOp::RelicShowcaseBatch(idx));
                    i += 1;
                }
                DrawCmd::PackBatch(placements) => {
                    let idx = pack_batches.len();
                    pack_batches.push(placements.as_slice());
                    ops.push(RenderOp::PackBatch(idx));
                    i += 1;
                }
                DrawCmd::DishExplicit(d) => {
                    let idx = aux_dish_cmds.len();
                    aux_dish_cmds.push(d);
                    ops.push(RenderOp::DishExplicit(idx));
                    i += 1;
                }
                DrawCmd::CurioCabinet(c) => {
                    cabinet_cmds.push(c);
                    ops.push(RenderOp::CurioCabinet);
                    i += 1;
                }
                DrawCmd::ShrineBatch(placements) => {
                    let idx = shrine_batches.len();
                    shrine_batches.push(placements.as_slice());
                    ops.push(RenderOp::ShrineBatch(idx));
                    i += 1;
                }
                DrawCmd::ZodiacBatch(placements) => {
                    let idx = ribbon_batches.len();
                    ribbon_batches.push(placements.as_slice());
                    ops.push(RenderOp::ZodiacBatch(idx));
                    i += 1;
                }
                DrawCmd::TalismanBatch(placements) => {
                    let idx = talisman_batches.len();
                    talisman_batches.push(placements.as_slice());
                    ops.push(RenderOp::TalismanBatch(idx));
                    i += 1;
                }
                DrawCmd::CoinBatch(placements) => {
                    let idx = coin_batches.len();
                    coin_batches.push(placements.as_slice());
                    ops.push(RenderOp::CoinBatch(idx));
                    i += 1;
                }
                DrawCmd::GoldBarBatch(placements) => {
                    let idx = bar_batches.len();
                    bar_batches.push(placements.as_slice());
                    ops.push(RenderOp::GoldBarBatch(idx));
                    i += 1;
                }
                DrawCmd::Book(p) => {
                    book_cmd = Some(p);
                    ops.push(RenderOp::Book);
                    i += 1;
                }
                DrawCmd::Plaque(p) => {
                    let idx = plaque_cmds.len();
                    plaque_cmds.push(p);
                    ops.push(RenderOp::Plaque(idx));
                    i += 1;
                }
                DrawCmd::Ofuda(p) => {
                    let idx = ofuda_cmds.len();
                    ofuda_cmds.push(p);
                    ops.push(RenderOp::Ofuda(idx));
                    i += 1;
                }
                DrawCmd::YakuTabletBatch(placements) => {
                    let idx = yaku_tablet_batches.len();
                    yaku_tablet_batches.push(placements.as_slice());
                    ops.push(RenderOp::YakuTabletBatch(idx));
                    i += 1;
                }
                DrawCmd::WoodTabletBatch(placements) => {
                    let idx = wood_tablet_batches.len();
                    wood_tablet_batches.push(placements.as_slice());
                    ops.push(RenderOp::WoodTabletBatch(idx));
                    i += 1;
                }
                DrawCmd::Bowl(p) => {
                    let idx = bowl_cmds.len();
                    bowl_cmds.push(p);
                    ops.push(RenderOp::Bowl(idx));
                    i += 1;
                }
                DrawCmd::Mirror(p) => {
                    let idx = mirror_cmds.len();
                    mirror_cmds.push(p);
                    ops.push(RenderOp::Mirror(idx));
                    i += 1;
                }
                DrawCmd::TallyFan(p) => {
                    let idx = tally_fan_cmds.len();
                    tally_fan_cmds.push(p);
                    ops.push(RenderOp::TallyFan(idx));
                    i += 1;
                }
                DrawCmd::WallStack(p) => {
                    let idx = wall_stack_cmds.len();
                    wall_stack_cmds.push(p);
                    ops.push(RenderOp::WallStack(idx));
                    i += 1;
                }
                DrawCmd::CascadeTokenBatch(placements) => {
                    let idx = cascade_token_batches.len();
                    cascade_token_batches.push(placements.as_slice());
                    ops.push(RenderOp::CascadeTokenBatch(idx));
                    i += 1;
                }
                DrawCmd::FallingBoneBatch(placements) => {
                    let idx = falling_bone_batches.len();
                    falling_bone_batches.push(placements.as_slice());
                    ops.push(RenderOp::FallingBoneBatch(idx));
                    i += 1;
                }
                DrawCmd::ExtrudedGlyphBatch(placements) => {
                    // Lazily build (and GPU-upload) the mesh for any label
                    // string we haven't seen before. Subsequent frames hit
                    // the `extruded_glyph_meshes` HashMap and skip both the
                    // tessellation and the buffer creation.
                    for p in placements.iter() {
                        if !self.extruded_glyph_meshes.contains_key(&p.label) {
                            if let Some(cpu) = self.glyph_cpu_cache.mesh_for(&p.label) {
                                let gpu = LitMeshGpu::new(
                                    &self.device,
                                    cpu,
                                    &format!("glyph-{}", p.label),
                                );
                                self.extruded_glyph_meshes.insert(p.label.clone(), gpu);
                            } else {
                                log::warn!(
                                    "[extruded glyph] mesh_for returned None for label {:?} — popup will be invisible",
                                    p.label
                                );
                            }
                        }
                    }
                    let idx = extruded_glyph_batches.len();
                    extruded_glyph_batches.push(placements.as_slice());
                    ops.push(RenderOp::ExtrudedGlyphBatch(idx));
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
                DrawCmd::Flame(_) => {
                    // Collect contiguous run of Flame cmds into a single batch.
                    // Each instance's screen-space rect is overridden with the
                    // pre-projected wick anchor for the matching candle, so the
                    // flame stays glued to the wick under camera perspective.
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::Flame(inst)) = frame.cmds.get(i) {
                        let mut fixed = *inst;
                        if let Some((rect, wind)) = flame_anchors.get(next_flame_anchor) {
                            fixed.rect = *rect;
                            // Pack the per-flame wind into color.rg
                            // (the slot the gameplay scene leaves at
                            // the unused [1,1,1] tint). The shader's
                            // current contract is rg=wind, b=reserved,
                            // a=phase — preserve the phase the scene
                            // baked in so neighbouring candles still
                            // flicker out of sync.
                            fixed.color[0] = wind[0];
                            fixed.color[1] = wind[1];
                            // Preserve the brightness the scene set in
                            // color[2] (flame shader's brightness channel).
                            next_flame_anchor += 1;
                        }
                        batch.push(fixed);
                        i += 1;
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("flame-batch"),
                            contents: bytemuck::cast_slice(&batch),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let buf_idx = flame_buffers.len();
                    flame_buffers.push(buf);
                    ops.push(RenderOp::FlameBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
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
                DrawCmd::RelicIcon(icon) => {
                    if self.relic_textures.contains_key(&icon.relic_id) {
                        let inst = GpuInstance {
                            rect: icon.rect,
                            color: [1.0, 1.0, 1.0, 1.0],
                        };
                        let inst_buf =
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("relic-icon-inst"),
                                    contents: bytemuck::cast_slice(&[inst]),
                                    usage: wgpu::BufferUsages::VERTEX,
                                });
                        let idx = image_draws.len();
                        image_draws.push(ImageDraw {
                            inst_buf,
                            bind_group: self.relic_textures[&icon.relic_id].bind_group.clone(),
                        });
                        ops.push(RenderOp::RelicIconDraw(idx));
                    }
                    i += 1;
                }
            }
        }

        // ── Debug axes overlay labels ───────────────────────────────────
        // After walking the scene's cmds, append three text labels (one per
        // axis) projected from the world-space tip of each debug-axes bar.
        // These get rasterized into ordinary text draws so they ride along
        // in the same render pass as the bars themselves.
        if frame.debug_axes {
            if let Some(ref font) = self.ui_font {
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

        // Candles: scenes pass `world_pos = (pixel_x, pixel_y, lift)` — we
        // map through [`pixel_to_world`]; `lift` is **+Z** above the felt.
        for batch in &candle_batches {
            for (slot_i, placement) in batch.iter().enumerate() {
                let Some(instances) = self.candle_instances.get(slot_i) else {
                    break;
                };
                let base = pixel_to_world(
                    w,
                    h,
                    placement.world_pos[0],
                    placement.world_pos[1],
                    placement.world_pos[2],
                );
                let s = placement.scale;
                let model = translate_rot_scale(
                    base,
                    mesh_y_thickness_along_local_y_to_z_up(),
                    glam::Vec3::new(s, s * placement.height_scale, s),
                );
                let candle_name = self.scene_path("candle");
                let model = self.apply_arrange_override(&candle_name, model);
                instances[0].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    self.candle_wax_mesh.default_material,
                );
                instances[1].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    self.candle_wick_mesh.default_material,
                );
                // Debug pick: candle wax body extends y ∈ [0, ~0.56],
                // wick tip at ~0.61, max radius ~0.36 (see candle_mesh.rs
                // WAX_PROFILE / WICK_TIP_Y). Half-Y = 0.305, center offset
                // = 0.305 so the box spans [0, 0.61].
                self.last_debug_pickables.push((
                    candle_name,
                    model,
                    glam::Vec3::new(0.36, 0.305, 0.36),
                    0.305,
                ));
            }
        }

        // ── Dish + relic placeholders ──────────────────────────────────
        // Each `RelicBatch` cmd carries a list of `RelicPlacement`s. We
        // turn each into a per-instance model matrix (translate + scale)
        // and project the resulting world-space bounding box back to
        // screen space so the scene layer can hit-test next frame.
        self.last_relic_models.clear();
        let mut dish_bounds: Option<(f32, f32, f32, f32)> = None;
        let mut relic_slot_cursor: usize = 0;
        for batch in &relic_batches {
            for p in batch.iter() {
                if relic_slot_cursor >= MAX_RELIC_SLOTS {
                    break;
                }
                let slot_i = relic_slot_cursor;
                relic_slot_cursor += 1;
                let center = pixel_to_world(
                    w,
                    h,
                    p.world_pos[0],
                    p.world_pos[1],
                    p.world_pos[2] + p.half_extents[1],
                );
                let rotation = rot_rx_rz_deg(p.rotation_x_deg, p.rotation_z_deg);
                let model = translate_rot_scale(
                    center,
                    rotation,
                    glam::Vec3::new(
                        p.half_extents[0] * 2.0,
                        p.half_extents[1] * 2.0,
                        p.half_extents[2] * 2.0,
                    ),
                );
                // `relic_batch` is used by gameplay (sidebar column) and shop
                // (all 4 for-sale stalls draw relics). Scene prefix disambiguates.
                let relic_name = match self.active_scene_key {
                    Some("shop") => "shop.for_sale.relics".to_string(),
                    Some("gameplay") => "gameplay.relic_col".to_string(),
                    _ => "relic".to_string(),
                };
                let model = self.apply_arrange_override(&relic_name, model);
                // Activation glow: lerp the rarity tint toward warm
                // champagne and push the brightness above 1.0 so the relic
                // visibly flares while a cascade step credits it. The
                // additive halo below adds the bloom around the silhouette.
                let g = p.glow.clamp(0.0, 1.0);
                let base_color = if g > 0.0 {
                    let target = [1.55, 1.32, 0.78, p.color[3]];
                    [
                        p.color[0] + (target[0] - p.color[0]) * g,
                        p.color[1] + (target[1] - p.color[1]) * g,
                        p.color[2] + (target[2] - p.color[2]) * g,
                        p.color[3],
                    ]
                } else {
                    p.color
                };
                let material = relic_material_params(p.relic_id, base_color, g);
                self.relic_instances[slot_i].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    material,
                );
                // Bind the relic's icon texture into the lit-mesh material
                // (same approach as zodiac ribbon textures). Skip the rebuild
                // when this slot already has the right texture bound.
                let want_tex: Option<RelicId> = if self.relic_textures.contains_key(&p.relic_id) {
                    Some(p.relic_id)
                } else {
                    None
                };
                if self.relic_slot_texture[slot_i] != want_tex {
                    let view: &wgpu::TextureView = match want_tex {
                        Some(rid) => &self.relic_textures[&rid].view,
                        None => &self.lit_mesh_white_view,
                    };
                    let relief_view: &wgpu::TextureView = match want_tex {
                        Some(rid) => &self.relic_textures[&rid].relief_view,
                        None => &self.lit_mesh_relief_default_view,
                    };
                    let inst = &mut self.relic_instances[slot_i];
                    inst.bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("relic-bg-tex"),
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
                                resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(relief_view),
                            },
                        ],
                    });
                    self.relic_slot_texture[slot_i] = want_tex;
                }
                self.last_relic_models.push(model);
                self.last_debug_pickables
                    .push((relic_name, model, glam::Vec3::splat(0.5), 0.0));

                // Project the box's 8 world corners to screen and take the
                // bounding rect — gives a 2D hit-test region the scene
                // can use for hover detection next frame.
                let hx = p.half_extents[0];
                let hy = p.half_extents[1];
                let hz = p.half_extents[2];
                let corners = [
                    glam::Vec3::new(-hx, -hy, -hz),
                    glam::Vec3::new(hx, -hy, -hz),
                    glam::Vec3::new(-hx, hy, -hz),
                    glam::Vec3::new(hx, hy, -hz),
                    glam::Vec3::new(-hx, -hy, hz),
                    glam::Vec3::new(hx, -hy, hz),
                    glam::Vec3::new(-hx, hy, hz),
                    glam::Vec3::new(hx, hy, hz),
                ];
                let mut mn_x = f32::INFINITY;
                let mut mn_y = f32::INFINITY;
                let mut mx_x = f32::NEG_INFINITY;
                let mut mx_y = f32::NEG_INFINITY;
                for c in corners {
                    let world = center + c;
                    let (sx, sy) = project_to_screen(world);
                    mn_x = mn_x.min(sx);
                    mn_y = mn_y.min(sy);
                    mx_x = mx_x.max(sx);
                    mx_y = mx_y.max(sy);
                }
                self.proj
                    .relic_rects
                    .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);

                // Activation halo: enqueue a champagne radial bloom around
                // the projected relic rect. `tile_glow_pipeline` does the
                // additive falloff; the rect is inflated so the halo spills
                // out past the box silhouette like the selected-tile glow.
                if g > 0.0 {
                    let rw_proj = mx_x - mn_x;
                    let rh_proj = mx_y - mn_y;
                    let pad_x = rw_proj * 0.85;
                    let pad_y = rh_proj * 0.95;
                    relic_glows.push(GpuInstance {
                        rect: [
                            mn_x - pad_x,
                            mn_y - pad_y,
                            rw_proj + pad_x * 2.0,
                            rh_proj + pad_y * 2.0,
                        ],
                        // Warm champagne; alpha encodes intensity. Slight
                        // boost above 1.0 lets the additive blend brighten
                        // even already-lit pixels around the relic.
                        color: [1.00, 0.82, 0.36, 1.20 * g],
                    });
                }

                // Track combined pixel-space extents so the dish auto-
                // sizes around the row.
                let px = p.world_pos[0];
                let py = p.world_pos[1];
                let pad = p.half_extents[0].max(p.half_extents[2]);
                let (lo_x, lo_y, hi_x, hi_y) = (
                    px - pad - 18.0,
                    py - pad - 18.0,
                    px + pad + 18.0,
                    py + pad + 18.0,
                );
                dish_bounds = Some(match dish_bounds {
                    None => (lo_x, lo_y, hi_x, hi_y),
                    Some((a, b, c, d)) => (a.min(lo_x), b.min(lo_y), c.max(hi_x), d.max(hi_y)),
                });
            }
        }

        // ── UI relic showcase viewers ─────────────────────────────────
        // Collection cards, tutorial panels, and modals render relics as
        // centered 3D objects using the same mesh/material path as the
        // physical relics. Projects screen-space rects into `relic_rects`
        // so scenes that use the showcase path for in-world badges (e.g.
        // the gameplay sidebar column) can hit-test them next frame.
        // In gameplay, the relic column is arrangeable via the debug
        // arrange tool (`gameplay.relic_col`). Apply a single group override
        // so the whole stack shifts/rotates as one unit, and track the
        // combined AABB so we can push a column-wide hit-box into
        // `last_debug_pickables` after the loop.
        let gameplay_relic_col = matches!(self.active_scene_key, Some("gameplay"));
        let mut relic_col_aabb: Option<(glam::Vec3, glam::Vec3)> = None;
        for batch in &relic_showcase_batches {
            for p in batch.iter() {
                if relic_slot_cursor >= MAX_RELIC_SLOTS {
                    break;
                }
                let slot_i = relic_slot_cursor;
                relic_slot_cursor += 1;
                let center =
                    pixel_to_world(w, h, p.center_pos[0], p.center_pos[1], p.center_pos[2]);
                // Relic showcase uses `Rx * Ry * Rz` — see `table_transform`.
                let rotation =
                    rot_rx_ry_rz_deg(p.rotation_x_deg, p.rotation_y_deg, p.rotation_z_deg);
                let model = translate_rot_scale(
                    center,
                    rotation,
                    glam::Vec3::new(p.extents[0], p.extents[1], p.extents[2]),
                );
                let model = if gameplay_relic_col {
                    self.apply_arrange_override("gameplay.relic_col", model)
                } else {
                    model
                };
                if gameplay_relic_col {
                    // Expand the shared column AABB by this relic's 8 corners
                    // in world space (post-arrange-override).
                    for c in [
                        glam::Vec3::new(-0.5, -0.5, -0.5),
                        glam::Vec3::new(0.5, -0.5, -0.5),
                        glam::Vec3::new(-0.5, 0.5, -0.5),
                        glam::Vec3::new(0.5, 0.5, -0.5),
                        glam::Vec3::new(-0.5, -0.5, 0.5),
                        glam::Vec3::new(0.5, -0.5, 0.5),
                        glam::Vec3::new(-0.5, 0.5, 0.5),
                        glam::Vec3::new(0.5, 0.5, 0.5),
                    ] {
                        let wp = (model * c.extend(1.0)).truncate();
                        relic_col_aabb = Some(match relic_col_aabb {
                            None => (wp, wp),
                            Some((mn, mx)) => (mn.min(wp), mx.max(wp)),
                        });
                    }
                }
                let g = p.glow.clamp(0.0, 1.0);
                let base_color = if g > 0.0 {
                    let target = [1.55, 1.32, 0.78, p.color[3]];
                    [
                        p.color[0] + (target[0] - p.color[0]) * g,
                        p.color[1] + (target[1] - p.color[1]) * g,
                        p.color[2] + (target[2] - p.color[2]) * g,
                        p.color[3],
                    ]
                } else {
                    p.color
                };
                self.relic_instances[slot_i].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    relic_material_params(p.relic_id, base_color, g),
                );
                let want_tex: Option<RelicId> = if self.relic_textures.contains_key(&p.relic_id) {
                    Some(p.relic_id)
                } else {
                    None
                };
                if self.relic_slot_texture[slot_i] != want_tex {
                    let view: &wgpu::TextureView = match want_tex {
                        Some(rid) => &self.relic_textures[&rid].view,
                        None => &self.lit_mesh_white_view,
                    };
                    let relief_view: &wgpu::TextureView = match want_tex {
                        Some(rid) => &self.relic_textures[&rid].relief_view,
                        None => &self.lit_mesh_relief_default_view,
                    };
                    let inst = &mut self.relic_instances[slot_i];
                    inst.bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("relic-showcase-bg-tex"),
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
                                resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(relief_view),
                            },
                        ],
                    });
                    self.relic_slot_texture[slot_i] = want_tex;
                }

                // Register the model so the bulk relic-rect rebuild below
                // projects this showcase slot into `proj.relic_rects` —
                // that's what focus-nav and cursor hit-testing read.
                self.last_relic_models.push(model);

                // Activation glow halo — project the unit-cube AABB now so
                // we can size the bloom to the actual on-screen footprint.
                if g > 0.0 {
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
                        let world = model.transform_point3(c);
                        let (sx, sy) = project_to_screen(world);
                        mn_x = mn_x.min(sx);
                        mn_y = mn_y.min(sy);
                        mx_x = mx_x.max(sx);
                        mx_y = mx_y.max(sy);
                    }
                    let rw_proj = mx_x - mn_x;
                    let rh_proj = mx_y - mn_y;
                    let pad_x = rw_proj * 0.85;
                    let pad_y = rh_proj * 0.95;
                    relic_glows.push(GpuInstance {
                        rect: [
                            mn_x - pad_x,
                            mn_y - pad_y,
                            rw_proj + pad_x * 2.0,
                            rh_proj + pad_y * 2.0,
                        ],
                        color: [1.00, 0.82, 0.36, 1.20 * g],
                    });
                }
            }
        }

        // Column-wide debug pickable so the arrange tool can grab the whole
        // relic stack via `gameplay.relic_col` (matching the tree entry in
        // scene_layout).  The hitbox is the union AABB of every relic's
        // post-arrange model, expressed in local-space with the identity
        // model (so the slab test reads world-space directly).
        if let Some((mn, mx)) = relic_col_aabb {
            let center = (mn + mx) * 0.5;
            let half = (mx - mn) * 0.5;
            let model = Mat4::from_translation(center);
            self.last_debug_pickables.push((
                "gameplay.relic_col".to_string(),
                model,
                half,
                0.0,
            ));
        }

        // ── Pack placeholders (same mesh + pipeline as relics) ──────────
        self.proj.pack_rects.clear();
        {
            let mut slot: usize = 0;
            for batch in &pack_batches {
                for p in batch.iter() {
                    if slot >= self.pack_instances.len() {
                        break;
                    }
                    let center = pixel_to_world(
                        w,
                        h,
                        p.world_pos[0],
                        p.world_pos[1],
                        p.world_pos[2] + p.half_extents[1],
                    );
                    let model = translate_rot_scale(
                        center,
                        rot_ry_rx_deg(p.rotation_x_deg, p.rotation_y_deg),
                        glam::Vec3::new(
                            p.half_extents[0] * 2.0,
                            p.half_extents[1] * 2.0,
                            p.half_extents[2] * 2.0,
                        ),
                    );
                    let material = MaterialParams {
                        kind: MaterialKind::Foil,
                        base_color: p.color,
                        specular_strength: 0.70,
                        specular_power: 48.0,
                    };
                    self.pack_instances[slot].write_uniform(
                        &self.queue,
                        view_proj_arr,
                        model,
                        material,
                    );
                    // Bind the pack's art texture.
                    let want_tex: Option<TilePackKind> = if self.pack_textures.contains_key(&p.kind)
                    {
                        Some(p.kind)
                    } else {
                        None
                    };
                    if self.pack_slot_texture[slot] != want_tex {
                        let view: &wgpu::TextureView = match want_tex {
                            Some(k) => &self.pack_textures[&k].view,
                            None => &self.lit_mesh_white_view,
                        };
                        let relief_view: &wgpu::TextureView = match want_tex {
                            Some(k) => &self.pack_textures[&k].relief_view,
                            None => &self.lit_mesh_relief_default_view,
                        };
                        let inst = &mut self.pack_instances[slot];
                        inst.bind_group =
                            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("pack-bg-tex"),
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
                                        resource: wgpu::BindingResource::TextureView(relief_view),
                                    },
                                ],
                            });
                        self.pack_slot_texture[slot] = want_tex;
                    }
                    // Project screen rect for hit-testing.
                    let hx = p.half_extents[0];
                    let hy = p.half_extents[1];
                    let hz = p.half_extents[2];
                    let corners = [
                        glam::Vec3::new(-hx, -hy, -hz),
                        glam::Vec3::new(hx, -hy, -hz),
                        glam::Vec3::new(-hx, hy, -hz),
                        glam::Vec3::new(hx, hy, -hz),
                        glam::Vec3::new(-hx, -hy, hz),
                        glam::Vec3::new(hx, -hy, hz),
                        glam::Vec3::new(-hx, hy, hz),
                        glam::Vec3::new(hx, hy, hz),
                    ];
                    let mut mn_x = f32::INFINITY;
                    let mut mn_y = f32::INFINITY;
                    let mut mx_x = f32::NEG_INFINITY;
                    let mut mx_y = f32::NEG_INFINITY;
                    for c in corners {
                        let world = center + c;
                        let (sx, sy) = project_to_screen(world);
                        mn_x = mn_x.min(sx);
                        mn_y = mn_y.min(sy);
                        mx_x = mx_x.max(sx);
                        mx_y = mx_y.max(sy);
                    }
                    self.proj
                        .pack_rects
                        .push(([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y], p.pick_id));
                    slot += 1;
                }
            }
        }
        // Dish: only sized when there are relics to hold. The dish is a
        // wide low box (width × thin height × depth in world units).
        let needs_dish = ops.iter().any(|o| matches!(o, RenderOp::Dish));
        if needs_dish {
            if let Some((lo_x, lo_y, hi_x, hi_y)) = dish_bounds {
                let cx = (lo_x + hi_x) * 0.5;
                let cy = (lo_y + hi_y) * 0.5;
                let dw = (hi_x - lo_x).max(40.0);
                let dd = (hi_y - lo_y).max(28.0);
                let dh = 10.0_f32; // dish rim height (world units)
                let center = pixel_to_world(w, h, cx, cy, dh * 0.5);
                let model =
                    translate_rot_scale(center, Mat4::IDENTITY, glam::Vec3::new(dw, dh, dd));
                self.dish_instance.write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    self.dish_mesh.default_material,
                );
            }
        }

        // ── Curio cabinet (single instance) ────────────────────────────
        self.last_cabinet_world_aabb = None;
        if let Some(c) = cabinet_cmds.first() {
            let center = pixel_to_world(w, h, c.center_pos[0], c.center_pos[1], c.center_pos[2]);
            let half = glam::Vec3::new(c.extents[0] * 0.5, c.extents[1] * 0.5, c.extents[2] * 0.5);
            let model = translate_rot_scale(
                center,
                Mat4::IDENTITY,
                glam::Vec3::new(c.extents[0], c.extents[1], c.extents[2]),
            );
            self.cabinet_instance.write_uniform(
                &self.queue,
                view_proj_arr,
                model,
                self.cabinet_mesh.default_material,
            );
            self.last_cabinet_world_aabb = Some((center, half));
        }

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

        // ── Auxiliary dishes (shop scene) ─────────────────────────────
        // Grow the GPU instance pool when more dishes are emitted than
        // previously allocated, so adding a new DishExplicit call can
        // never silently vanish.
        self.proj.aux_dish_rects.clear();
        self.last_aux_dish_aabbs.clear();
        while self.aux_dish_instances.len() < aux_dish_cmds.len() {
            self.aux_dish_instances.push(LitMeshInstance::new(
                &self.device,
                &self.lit_mesh_material_layout,
                &self.shadow_caster_layout,
                &self.lit_mesh_white_view,
                &self.lit_mesh_relief_default_view,
                &self.tile_sampler,
            ));
        }
        for (slot_i, d) in aux_dish_cmds.iter().enumerate() {
            let center = pixel_to_world(
                w,
                h,
                d.center_pos[0],
                d.center_pos[1],
                d.center_pos[2] + d.extents[1] * 0.5,
            );
            // Round dishes use a Y-up mesh; rotate local Y into world Z so the
            // rim sits flat on the table. Square dishes are already authored
            // with their thickness on local Y mapped naturally by callers.
            let oriented = if d.round {
                mesh_y_thickness_along_local_y_to_z_up() * d.rotation
            } else {
                d.rotation
            };
            let model = translate_rot_scale(
                center,
                oriented,
                glam::Vec3::new(d.extents[0], d.extents[1], d.extents[2]),
            );
            let model = if let Some(name) = d.arrange_name {
                self.apply_arrange_override(name, model)
            } else {
                model
            };
            self.aux_dish_instances[slot_i].write_uniform(
                &self.queue,
                view_proj_arr,
                model,
                self.dish_mesh.default_material,
            );
            if let Some(name) = d.arrange_name {
                // The `model` matrix already bakes in the full extents as its
                // scale, so local space spans [-0.5, 0.5]^3 — matching every
                // other Object3d entry in `last_debug_pickables`.  Using
                // extents/2 here would give a quadratically-inflated hitbox
                // (world overlap ≈ extents^2/2) that catches clicks far outside
                // the dish's visual bounds.
                self.last_debug_pickables.push((
                    name.to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
            }
            // Project the dish AABB to a screen rect for the scene's
            // hover overlays + cursor pick.
            let hx = d.extents[0] * 0.5;
            let hy = d.extents[1] * 0.5;
            let hz = d.extents[2] * 0.5;
            let half = glam::Vec3::new(hx, hy, hz);
            let mut mn_x = f32::INFINITY;
            let mut mn_y = f32::INFINITY;
            let mut mx_x = f32::NEG_INFINITY;
            let mut mx_y = f32::NEG_INFINITY;
            for sx in [-hx, hx] {
                for sy in [-hy, hy] {
                    for sz in [-hz, hz] {
                        let world = center + glam::Vec3::new(sx, sy, sz);
                        let (px, py) = project_to_screen(world);
                        mn_x = mn_x.min(px);
                        mn_y = mn_y.min(py);
                        mx_x = mx_x.max(px);
                        mx_y = mx_y.max(py);
                    }
                }
            }
            self.proj
                .aux_dish_rects
                .push((d.pick_id, [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]));
            self.last_aux_dish_aabbs.push((center, half));
        }

        // ── Ribbon batches (shop scene) ────────────────────────────────
        // Each textured ribbon uses up to 3 draw slots (top cap, tileable
        // middle, bottom cap) so its length is independent of texture aspect.
        // Untextured (plain) ribbons still use a single slot.
        self.last_ribbon_models.clear();
        self.last_ribbon_batch_slot_counts.clear();
        let mut ribbon_slot_cursor: usize = 0;
        for batch in &ribbon_batches {
            let batch_start = ribbon_slot_cursor;
            for r in batch.iter() {
                if ribbon_slot_cursor >= MAX_RIBBON_SLOTS {
                    break;
                }
                let anchor =
                    pixel_to_world(w, h, r.anchor_pos[0], r.anchor_pos[1], r.anchor_pos[2]);
                let eff_w = r.width;
                let eff_l = r.length;
                let depth = eff_w * 0.15;
                let base_transform = translate_rot_scale(
                    anchor,
                    rot_rz_ry_rx_deg(r.rotation_x_deg, r.rotation_y_deg, r.rotation_z_deg),
                    glam::Vec3::splat(1.0),
                );
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: r.color,
                    specular_strength: 0.25,
                    specular_power: 16.0,
                };

                let zod_idx: Option<u8> = r.kind.map(|z| {
                    crate::core::zodiac::ZodiacKind::all()
                        .iter()
                        .position(|kk| *kk == z)
                        .unwrap_or(0) as u8
                });

                // Helper: bind a texture to a ribbon slot and write its uniform.
                let mut emit_slot = |slot_i: usize, model: Mat4, want: Option<(u8, u8)>| {
                    if self.ribbon_slot_zodiac[slot_i] != want {
                        let view: &wgpu::TextureView = match want {
                            Some((idx, 0)) => &self.ribbon_zodiac_tex.top_views[idx as usize],
                            Some((idx, 1)) => &self.ribbon_zodiac_tex.mid_views[idx as usize],
                            Some((idx, _)) => &self.ribbon_zodiac_tex.bot_views[idx as usize],
                            None => &self.lit_mesh_white_view,
                        };
                        let inst = &mut self.ribbon_instances[slot_i];
                        inst.bind_group =
                            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("ribbon-bg-zodiac"),
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
                                            &self.lit_mesh_relief_default_view,
                                        ),
                                    },
                                ],
                            });
                        self.ribbon_slot_zodiac[slot_i] = want;
                    }
                    self.ribbon_instances[slot_i].write_uniform(
                        &self.queue,
                        view_proj_arr,
                        model,
                        material,
                    );
                };

                if let Some(idx) = zod_idx {
                    // Textured ribbon — three parts: top cap, tileable mid, bot cap.
                    // Each cap is a fixed fraction of the ribbon width so
                    // the middle section is always visible, even on short
                    // ribbons. Clamp when the ribbon is extremely short.
                    let nominal_cap = eff_w * 0.33;
                    let cap_h = if eff_l < 2.0 * nominal_cap {
                        eff_l / 2.0
                    } else {
                        nominal_cap
                    };
                    let mid_h = (eff_l - 2.0 * cap_h).max(0.0);
                    let slots_needed = if mid_h > 0.0 { 3 } else { 2 };
                    if ribbon_slot_cursor + slots_needed > MAX_RIBBON_SLOTS {
                        break;
                    }

                    // Top cap
                    let top_model =
                        ribbon_submesh(base_transform, 0.0, glam::Vec3::new(eff_w, cap_h, depth));
                    emit_slot(ribbon_slot_cursor, top_model, Some((idx, 0)));
                    ribbon_slot_cursor += 1;

                    // Middle (stretches to fill remaining length)
                    if mid_h > 0.0 {
                        let mid_model = ribbon_submesh(
                            base_transform,
                            -cap_h,
                            glam::Vec3::new(eff_w, mid_h, depth),
                        );
                        emit_slot(ribbon_slot_cursor, mid_model, Some((idx, 1)));
                        ribbon_slot_cursor += 1;
                    }

                    // Bottom cap
                    let bot_model = ribbon_submesh(
                        base_transform,
                        -(cap_h + mid_h),
                        glam::Vec3::new(eff_w, cap_h, depth),
                    );
                    emit_slot(ribbon_slot_cursor, bot_model, Some((idx, 2)));
                    ribbon_slot_cursor += 1;

                    // For pick-testing, store the full-ribbon model matrix.
                    let full_model =
                        ribbon_submesh(base_transform, 0.0, glam::Vec3::new(eff_w, eff_l, depth));
                    self.last_ribbon_models.push(full_model);
                } else {
                    // Untextured (plain) ribbon — single slot, same as before.
                    let model =
                        ribbon_submesh(base_transform, 0.0, glam::Vec3::new(eff_w, eff_l, depth));
                    emit_slot(ribbon_slot_cursor, model, None);
                    ribbon_slot_cursor += 1;
                    self.last_ribbon_models.push(model);
                }

                // Project the ribbon's full AABB to screen for tooltip/click.
                let full_model =
                    ribbon_submesh(base_transform, 0.0, glam::Vec3::new(eff_w, eff_l, depth));
                let local_corners = [
                    glam::Vec3::new(-0.5, -1.0, -0.05),
                    glam::Vec3::new(0.5, -1.0, -0.05),
                    glam::Vec3::new(-0.5, 0.0, -0.05),
                    glam::Vec3::new(0.5, 0.0, -0.05),
                    glam::Vec3::new(-0.5, -1.0, 0.05),
                    glam::Vec3::new(0.5, -1.0, 0.05),
                    glam::Vec3::new(-0.5, 0.0, 0.05),
                    glam::Vec3::new(0.5, 0.0, 0.05),
                ];
                let mut mn_x = f32::INFINITY;
                let mut mn_y = f32::INFINITY;
                let mut mx_x = f32::NEG_INFINITY;
                let mut mx_y = f32::NEG_INFINITY;
                for c in local_corners {
                    let w_pt = full_model.transform_point3(c);
                    let (sx, sy) = project_to_screen(w_pt);
                    mn_x = mn_x.min(sx);
                    mn_y = mn_y.min(sy);
                    mx_x = mx_x.max(sx);
                    mx_y = mx_y.max(sy);
                }
                self.proj
                    .ribbon_rects
                    .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
            }
            self.last_ribbon_batch_slot_counts
                .push(ribbon_slot_cursor - batch_start);
        }
        self.last_ribbon_slot_count = ribbon_slot_cursor;

        // ── Talisman batches (shop scene) ──────────────────────────────
        self.last_talisman_models.clear();
        let mut talisman_slot_cursor: usize = 0;
        for batch in &talisman_batches {
            for t in batch.iter() {
                if talisman_slot_cursor >= MAX_TALISMAN_SLOTS {
                    break;
                }
                let slot_i = talisman_slot_cursor;
                talisman_slot_cursor += 1;
                let center =
                    pixel_to_world(w, h, t.center_pos[0], t.center_pos[1], t.center_pos[2]);
                // Talisman mesh local extents are (HALF_W, HALF_H, HALF_T) ≈
                // (0.5, 0.7, 0.09); scale so the world-space bounds match
                // the requested extents.
                let sx = t.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                let sy = t.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                let sz = t.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                let model = translate_rot_scale(
                    center,
                    rot_rz_ry_rx_deg(t.rotation_x_deg, t.rotation_y_deg, t.rotation_z_deg),
                    glam::Vec3::new(sx, sy, sz),
                );
                let material = talisman_material(t.kind, t.color);
                // Rebind the heightmap texture if this slot's kind changed.
                let kind_idx = crate::core::talisman::TalismanKind::all()
                    .iter()
                    .position(|&k| k == t.kind)
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
                    model,
                    material,
                    kind_idx as f32,
                );
                self.last_talisman_models.push(model);

                // Project local AABB to screen for the tooltip anchor.
                let hx = TALISMAN_LOCAL_HALF[0];
                let hy = TALISMAN_LOCAL_HALF[1];
                let hz = TALISMAN_LOCAL_HALF[2];
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
                let mut mn_x = f32::INFINITY;
                let mut mn_y = f32::INFINITY;
                let mut mx_x = f32::NEG_INFINITY;
                let mut mx_y = f32::NEG_INFINITY;
                for c in local_corners {
                    let w_pt = model.transform_point3(c);
                    let (psx, psy) = project_to_screen(w_pt);
                    mn_x = mn_x.min(psx);
                    mn_y = mn_y.min(psy);
                    mx_x = mx_x.max(psx);
                    mx_y = mx_y.max(psy);
                }
                self.proj
                    .talisman_rects
                    .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
            }
        }

        // ── Coin batches (shop scene) ──────────────────────────────────
        let mut coin_slot_cursor: usize = 0;
        for batch in &coin_batches {
            for c in batch.iter() {
                if coin_slot_cursor >= MAX_COIN_SLOTS {
                    break;
                }
                let slot_i = coin_slot_cursor;
                coin_slot_cursor += 1;
                let center = pixel_to_world(w, h, c.world_pos[0], c.world_pos[1], c.world_pos[2]);
                let model = translate_rot_scale(
                    center,
                    rot_z_rad(c.rotation_y) * mesh_y_thickness_along_local_y_to_z_up(),
                    glam::Vec3::new(c.radius * 2.0, c.thickness, c.radius * 2.0),
                );
                let material = MaterialParams {
                    kind: MaterialKind::Metal,
                    base_color: c.color,
                    specular_strength: 1.0,
                    specular_power: 96.0,
                };
                self.coin_instances[slot_i].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    material,
                );
            }
        }

        // ── Gold bar batches (shop scene) ─────────────────────────────
        let mut bar_slot_cursor: usize = 0;
        for batch in &bar_batches {
            for b in batch.iter() {
                if bar_slot_cursor >= MAX_BAR_SLOTS {
                    break;
                }
                let slot_i = bar_slot_cursor;
                bar_slot_cursor += 1;
                let center = pixel_to_world(w, h, b.world_pos[0], b.world_pos[1], b.world_pos[2]);
                let model = translate_rot_scale(
                    center,
                    rot_z_rad(b.rotation_y),
                    glam::Vec3::new(
                        b.half_extents[0] * 2.0,
                        b.half_extents[1] * 2.0,
                        b.half_extents[2] * 2.0,
                    ),
                );
                let material = MaterialParams {
                    kind: MaterialKind::Metal,
                    base_color: b.color,
                    specular_strength: 1.0,
                    specular_power: 96.0,
                };
                self.bar_instances[slot_i].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    material,
                );
            }
        }

        // ── Book (journal bookend, shop scene) ──────────────────────────
        self.last_book_model = None;
        self.last_ofuda_model = None;
        self.last_info_plaque_model = None;
        self.last_leave_prop_model = None;
        self.last_reroll_prop_model = None;
        self.last_sell_tray_model = None;
        self.last_sell_card_model = None;
        if let Some(b) = book_cmd {
            let center = pixel_to_world(w, h, b.world_pos[0], b.world_pos[1], b.world_pos[2]);
            let model = translate_rot_scale(
                center,
                rot_z_rad(b.rotation_y),
                glam::Vec3::new(
                    b.half_extents[0] * 2.0,
                    b.half_extents[1] * 2.0,
                    b.half_extents[2] * 2.0,
                ),
            );
            let material = MaterialParams {
                kind: MaterialKind::Plain,
                base_color: b.color,
                specular_strength: 0.20,
                specular_power: 16.0,
            };
            self.book_instance
                .write_uniform(&self.queue, view_proj_arr, model, material);
            // Store model matrix + pick id for local-space slab-test picking.
            if let Some(pid) = b.pick_id {
                self.last_book_model = Some((model, pid));
                // Project the book's AABB to a screen rect so the shop's
                // focus-rect graph can reach it via keyboard / controller.
                let hx = b.half_extents[0];
                let hy = b.half_extents[1];
                let hz = b.half_extents[2];
                let mut mn_x = f32::INFINITY;
                let mut mn_y = f32::INFINITY;
                let mut mx_x = f32::NEG_INFINITY;
                let mut mx_y = f32::NEG_INFINITY;
                for sx in [-hx, hx] {
                    for sy in [-hy, hy] {
                        for sz in [-hz, hz] {
                            let w = center + glam::Vec3::new(sx, sy, sz);
                            let (px, py) = project_to_screen(w);
                            mn_x = mn_x.min(px);
                            mn_y = mn_y.min(py);
                            mx_x = mx_x.max(px);
                            mx_y = mx_y.max(py);
                        }
                    }
                }
                self.proj
                    .aux_dish_rects
                    .push((Some(pid), [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]));
            }
        }

        // ── Skeuomorphic gameplay HUD uniform writes (phase 1) ─────────
        //
        // The new HUD meshes (plaque, ofuda, tablets, bowl, peg block, wall
        // stack) all share the lit-mesh pipeline. Each gets its
        // own slot pool above; per-frame we walk the cmds, write the
        // per-instance uniform, and (where the scene needs it for hit
        // testing in later phases) project the AABB to a screen-space rect.
        self.proj.yaku_tablet_rects.clear();
        self.proj.wood_tablet_rects.clear();
        self.proj.bowl_rect = None;
        self.proj.mirror_rect = None;
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
        //
        // Derive plaque tilt from the active camera the same way wood tablets
        // do, so the face always reads correctly regardless of scene.
        let plaque_tilt_deg = {
            let look = look_target - cam_pos;
            look.z.atan2(look.y.abs()).to_degrees() + 180.0
        };
        self.proj.plaque_rects.clear();
        ensure_lit_mesh_pool(
            &mut self.plaque_instances,
            plaque_cmds.len(),
            &self.device,
            &self.lit_mesh_material_layout,
            &self.shadow_caster_layout,
            &self.lit_mesh_white_view,
            &self.lit_mesh_relief_default_view,
            &self.tile_sampler,
        );
        for (slot_i, p) in plaque_cmds.iter().enumerate() {
            let center = pixel_to_world(w, h, p.center_pos[0], p.center_pos[1], p.center_pos[2]);
            // Plaque pipeline is gameplay-only (boss plaque + scoring placard).
            let plaque_name = if slot_i == 0 {
                "gameplay.score_panel.plaque"
            } else {
                "gameplay.score_panel.scoring_placard"
            };
            let model = translate_rot_scale(
                center,
                rot_rz_rx_deg(plaque_tilt_deg, p.rotation_y_deg),
                glam::Vec3::new(p.extents[0], p.extents[1], p.extents[2]),
            );
            let model = self.apply_arrange_override(plaque_name, model);
            // Engraved two-line decal painted on the +Z face. Empty top
            // *and* empty bottom = no decal needed (the second placard
            // plaque uses this path with no engraved text). Otherwise
            // rasterize once when either line changes and treat the
            // texture as a transparent overlay via `has_decal = true`.
            let has_decal_text = !p.text.trim().is_empty();
            if has_decal_text {
                // Size the decal texture to match the plaque face's actual
                // aspect ratio so the bilinear sampler maps texels 1:1 onto
                // the wood — otherwise wide faces stretch glyphs horizontally
                // and tall faces squash them. Pin a reference height and
                // derive the width from `extents[0] / extents[1]`, clamped
                // so we never blow GPU memory on extreme aspect windows.
                let decal_h = crate::render::decal::PLAQUE_DECAL_HEIGHT;
                let face_aspect = (p.extents[0] / p.extents[1].max(1.0)).clamp(0.5, 12.0);
                let decal_w = ((decal_h as f32 * face_aspect).round() as u32).clamp(256, 4096);
                let label_hash = tablet_label_hash(&p.text, decal_w, decal_h);
                let inst = &mut self.plaque_instances[slot_i];
                if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                    let rgba = crate::render::decal::rasterize_plaque_decal(
                        &p.text,
                        self.ui_font.as_ref(),
                        decal_w,
                        decal_h,
                    );
                    inst.set_decal(
                        &self.device,
                        &self.queue,
                        &self.lit_mesh_material_layout,
                        &self.tile_sampler,
                        &self.lit_mesh_relief_default_view,
                        &rgba,
                        decal_w,
                        decal_h,
                    );
                    inst.decal_label_hash = label_hash;
                }
            }
            self.plaque_instances[slot_i].write_uniform_with_decal(
                &self.queue,
                view_proj_arr,
                model,
                self.plaque_mesh.default_material,
                has_decal_text,
            );
            // Project the slab face (front +Z) corners to screen so the
            // scene can overlay 2D text aligned with the rendered plaque.
            // Use only the front-face slab corners — the chain nubs poke
            // above the top edge and would skew the bounding box upward.
            let face_corners = [
                glam::Vec3::new(-0.5, -0.5, 0.5),
                glam::Vec3::new(0.5, -0.5, 0.5),
                glam::Vec3::new(-0.5, 0.5, 0.5),
                glam::Vec3::new(0.5, 0.5, 0.5),
            ];
            let mut mn_x = f32::INFINITY;
            let mut mn_y = f32::INFINITY;
            let mut mx_x = f32::NEG_INFINITY;
            let mut mx_y = f32::NEG_INFINITY;
            for c in face_corners {
                let w_pt = model.transform_point3(c);
                let (sx, sy) = project_to_screen(w_pt);
                mn_x = mn_x.min(sx);
                mn_y = mn_y.min(sy);
                mx_x = mx_x.max(sx);
                mx_y = mx_y.max(sy);
            }
            self.proj
                .plaque_rects
                .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
            // Plaque local AABB is the unit cube (push_box convention).
            self.last_debug_pickables.push((
                plaque_name.to_string(),
                model,
                glam::Vec3::splat(0.5),
                0.0,
            ));
        }

        // Ofuda (single instance per cmd). The paper hangs upright on the
        // back wall — no camera-toward tilt — so the wrapped rule decal
        // reads as a posted notice instead of foreshortening into a sliver.
        let ofuda_tilt_x = 0.0_f32.to_radians();
        for (slot_i, p) in ofuda_cmds.iter().enumerate() {
            if slot_i >= MAX_OFUDA_SLOTS {
                break;
            }
            let center = pixel_to_world(w, h, p.center_pos[0], p.center_pos[1], p.center_pos[2]);
            let model = translate_rot_scale(
                center,
                rot_ry_rx_deg(ofuda_tilt_x.to_degrees(), p.rotation_y_deg),
                glam::Vec3::new(p.extents[0], p.extents[1], p.extents[2]),
            );
            let model = self.apply_arrange_override("gameplay.score_panel.ofuda", model);
            // Boss-rule decal: rasterise the title + wrapped rule onto a
            // portrait texture and bind it as the per-instance albedo
            // overlay so the +Z paper face shows the calligraphy. Cached
            // by combined-text hash so steady-state cost is one compare.
            let has_decal_text = !p.title.is_empty() || !p.rule.is_empty();
            if has_decal_text {
                // Size the decal texture to the paper face's actual aspect
                // ratio so the bilinear sampler maps texels 1:1 onto the
                // ofuda. The ofuda is portrait, so we pin the long edge as
                // the *height* and derive width from `extents[0] /
                // extents[1]`. Clamp the aspect so a degenerate face can't
                // request a runaway texture allocation.
                let decal_h = crate::render::decal::OFUDA_DECAL_LONG_EDGE;
                let face_aspect = (p.extents[0] / p.extents[1].max(1.0)).clamp(0.1, 4.0);
                let decal_w = ((decal_h as f32 * face_aspect).round() as u32).clamp(128, 4096);
                let combined = format!("{}\n{}", p.title, p.rule);
                let label_hash = tablet_label_hash(&combined, decal_w, decal_h);
                let inst = &mut self.ofuda_instances[slot_i];
                if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                    let rgba = crate::render::decal::rasterize_ofuda_decal(
                        &p.title,
                        &p.rule,
                        self.ui_font.as_ref(),
                        decal_w,
                        decal_h,
                    );
                    inst.set_decal(
                        &self.device,
                        &self.queue,
                        &self.lit_mesh_material_layout,
                        &self.tile_sampler,
                        &self.lit_mesh_relief_default_view,
                        &rgba,
                        decal_w,
                        decal_h,
                    );
                    inst.decal_label_hash = label_hash;
                }
            }
            self.ofuda_instances[slot_i].write_uniform_with_decal(
                &self.queue,
                view_proj_arr,
                model,
                self.ofuda_mesh.default_material,
                has_decal_text,
            );
            self.last_debug_pickables.push((
                "gameplay.score_panel.ofuda".to_string(),
                model,
                glam::Vec3::splat(0.5),
                0.0,
            ));
        }

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
                    specular_strength: 0.65 + 0.20 * t.hover.clamp(0.0, 1.0),
                    specular_power: 96.0,
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
                        &self.device,
                        &self.queue,
                        &self.lit_mesh_material_layout,
                        &self.tile_sampler,
                        &self.lit_mesh_relief_default_view,
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

        // Wood action tablets (sort suit / sort rank / play).
        let mut wood_tablet_slot_cursor: usize = 0;
        for batch in &wood_tablet_batches {
            for t in batch.iter() {
                if wood_tablet_slot_cursor >= MAX_WOOD_TABLET_SLOTS {
                    break;
                }
                let slot_i = wood_tablet_slot_cursor;
                wood_tablet_slot_cursor += 1;
                let lift = t.hover.clamp(0.0, 1.0) * t.extents[1] * 0.25;
                let press = t.pressed.clamp(0.0, 1.0) * t.extents[1] * 0.3;
                let center = pixel_to_world(
                    w,
                    h,
                    t.world_pos[0],
                    t.world_pos[1],
                    t.world_pos[2] + t.extents[1] * 0.5 + lift - press,
                );
                // Tilt the top face toward the camera so the engraved
                // label is readable. Derive the pitch from the active camera
                // so the tilt is correct for both the gameplay camera and
                // custom cameras (e.g. the main-menu start screen).
                let look = look_target - cam_pos;
                let tilt_deg = look.z.atan2(look.y.abs()).to_degrees() + 180.0;
                // Wood tablets are a gameplay-only action-bar element. Slot
                // order matches the scene's push order in gameplay.rs (sort
                // suit, sort rank, cash-in/trigger, journal).
                let wood_tablet_name = match slot_i {
                    0 => "gameplay.action_bar.tablet_sort_suit",
                    1 => "gameplay.action_bar.tablet_sort_rank",
                    2 => "gameplay.action_bar.tablet_cash_in",
                    3 => "gameplay.action_bar.tablet_journal",
                    _ => "gameplay.action_bar.tablet",
                };
                let model = translate_rot_scale(
                    center,
                    rot_rz_rx_deg(tilt_deg, t.rotation_z_deg),
                    glam::Vec3::new(t.extents[0], t.extents[1], t.extents[2]),
                );
                let model = self.apply_arrange_override(wood_tablet_name, model);
                let label_hash = tablet_label_hash(&t.label, 512, 192);
                let inst = &mut self.wood_tablet_instances[slot_i];
                if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                    let rgba = crate::render::decal::rasterize_wood_tablet_decal(
                        &t.label,
                        self.ui_font.as_ref(),
                    );
                    inst.set_decal(
                        &self.device,
                        &self.queue,
                        &self.lit_mesh_material_layout,
                        &self.tile_sampler,
                        &self.lit_mesh_relief_default_view,
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
                    wood_tablet_name.to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
            }
        }

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

            let mut obj3d_plaque_slot: usize = 0;
            let mut obj3d_ofuda_slot: usize = 0;
            let mut obj3d_yaku_slot: usize = 0;
            let mut obj3d_wood_slot: usize = 0;
            let mut obj3d_relic_slot: usize = 0;
            let mut obj3d_pack_slot: usize = 0;
            let mut obj3d_talisman_slot: usize = 0;
            let mut obj3d_ribbon_slot: usize = 0;
            let mut obj3d_coin_slot: usize = 0;
            let mut obj3d_shop_action_prop_slot: usize = 0;
            let mut obj3d_bar_slot: usize = 0;
            let mut obj3d_shrine_slot: usize = 0;
            let mut obj3d_orb_slot: usize = 0;
            let mut _obj3d_bowl_slot: usize = 0;
            let mut _obj3d_mirror_slot: usize = 0;

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
                        Object3dKind::Plaque { text, pick_id } => {
                            let slot_i = obj3d_plaque_slot;
                            obj3d_plaque_slot += 1;
                            ensure_lit_mesh_pool(
                                &mut self.plaque_instances,
                                slot_i + 1,
                                &self.device,
                                &self.lit_mesh_material_layout,
                                &self.shadow_caster_layout,
                                &self.lit_mesh_white_view,
                                &self.lit_mesh_relief_default_view,
                                &self.tile_sampler,
                            );
                            let has_decal = !text.trim().is_empty();
                            if has_decal {
                                let decal_h = crate::render::decal::PLAQUE_DECAL_HEIGHT;
                                let face_aspect =
                                    (obj.extents[0] / obj.extents[1].max(1.0)).clamp(0.5, 12.0);
                                let decal_w = ((decal_h as f32 * face_aspect).round() as u32)
                                    .clamp(256, 4096);
                                let label_hash = tablet_label_hash(text, decal_w, decal_h);
                                let inst = &mut self.plaque_instances[slot_i];
                                if inst.decal_texture.is_none()
                                    || inst.decal_label_hash != label_hash
                                {
                                    let rgba = crate::render::decal::rasterize_plaque_decal(
                                        text,
                                        self.ui_font.as_ref(),
                                        decal_w,
                                        decal_h,
                                    );
                                    inst.set_decal(
                                        &self.device,
                                        &self.queue,
                                        &self.lit_mesh_material_layout,
                                        &self.tile_sampler,
                                        &self.lit_mesh_relief_default_view,
                                        &rgba,
                                        decal_w,
                                        decal_h,
                                    );
                                    inst.decal_label_hash = label_hash;
                                }
                            }
                            // Object3dKind::Plaque is used by shop (info plaque)
                            // and gameplay (blind plaque / scoring placard);
                            // disambiguate by scene. When the scene supplies an
                            // explicit arrange_name, honor it so multiple
                            // plaques in one scene can be placed independently.
                            let plaque_name = if let Some(name) = obj.arrange_name {
                                name.to_string()
                            } else {
                                match (self.active_scene_key, slot_i) {
                                    (Some("gameplay"), 0) => "gameplay.score_panel.plaque".to_string(),
                                    (Some("gameplay"), 1) => {
                                        "gameplay.score_panel.scoring_placard".to_string()
                                    }
                                    (Some("shop"), i) => format!("shop.plaque[{i}]"),
                                    (_, i) => format!("plaque[{i}]"),
                                }
                            };
                            let model = self.apply_arrange_override(&plaque_name, model);
                            if let Some(pid) = pick_id {
                                self.last_info_plaque_model = Some((model, *pid));
                            }
                            self.plaque_instances[slot_i].write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                self.plaque_mesh.default_material,
                                has_decal,
                            );
                            self.last_debug_pickables.push((
                                plaque_name,
                                model,
                                glam::Vec3::new(0.5, 0.5, 0.1),
                                0.0,
                            ));
                            let face_corners = [
                                glam::Vec3::new(-0.5, -0.5, 0.5),
                                glam::Vec3::new(0.5, -0.5, 0.5),
                                glam::Vec3::new(-0.5, 0.5, 0.5),
                                glam::Vec3::new(0.5, 0.5, 0.5),
                            ];
                            let mut mn_x = f32::INFINITY;
                            let mut mn_y = f32::INFINITY;
                            let mut mx_x = f32::NEG_INFINITY;
                            let mut mx_y = f32::NEG_INFINITY;
                            for c in face_corners {
                                let w_pt = model.transform_point3(c);
                                let (sx, sy) = project_to_screen(w_pt);
                                mn_x = mn_x.min(sx);
                                mn_y = mn_y.min(sy);
                                mx_x = mx_x.max(sx);
                                mx_y = mx_y.max(sy);
                            }
                            self.proj
                                .plaque_rects
                                .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
                            object3d_draw_list.push((0, slot_i));
                        }
                        Object3dKind::Ofuda {
                            title,
                            rule,
                            pick_id,
                        } => {
                            let slot_i = obj3d_ofuda_slot;
                            obj3d_ofuda_slot += 1;
                            if slot_i >= MAX_OFUDA_SLOTS {
                                continue;
                            }
                            let has_decal = !title.is_empty() || !rule.is_empty();
                            if has_decal {
                                let decal_h = crate::render::decal::OFUDA_DECAL_LONG_EDGE;
                                let face_aspect =
                                    (obj.extents[0] / obj.extents[1].max(1.0)).clamp(0.1, 4.0);
                                let decal_w = ((decal_h as f32 * face_aspect).round() as u32)
                                    .clamp(128, 4096);
                                let combined = format!("{}\n{}", title, rule);
                                let label_hash = tablet_label_hash(&combined, decal_w, decal_h);
                                let inst = &mut self.ofuda_instances[slot_i];
                                if inst.decal_texture.is_none()
                                    || inst.decal_label_hash != label_hash
                                {
                                    let rgba = crate::render::decal::rasterize_ofuda_decal(
                                        title,
                                        rule,
                                        self.ui_font.as_ref(),
                                        decal_w,
                                        decal_h,
                                    );
                                    inst.set_decal(
                                        &self.device,
                                        &self.queue,
                                        &self.lit_mesh_material_layout,
                                        &self.tile_sampler,
                                        &self.lit_mesh_relief_default_view,
                                        &rgba,
                                        decal_w,
                                        decal_h,
                                    );
                                    inst.decal_label_hash = label_hash;
                                }
                            }
                            // Object3dKind::Ofuda fires for shop's entry-scroll
                            // and gameplay's boss rule card. Honor an explicit
                            // arrange_name when the scene provides one; otherwise
                            // fall back to the scene-key convention.
                            let ofuda_name = if let Some(name) = obj.arrange_name {
                                name.to_string()
                            } else {
                                match self.active_scene_key {
                                    Some("shop") => "shop.props.ofuda".to_string(),
                                    Some("gameplay") => "gameplay.score_panel.ofuda".to_string(),
                                    _ => format!("ofuda[{slot_i}]"),
                                }
                            };
                            let model = self.apply_arrange_override(&ofuda_name, model);
                            if let Some(pid) = pick_id {
                                self.last_ofuda_model = Some((model, *pid));
                            }
                            self.ofuda_instances[slot_i].write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                self.ofuda_mesh.default_material,
                                has_decal,
                            );
                            self.last_debug_pickables.push((
                                ofuda_name,
                                model,
                                glam::Vec3::new(0.5, 0.5, 0.1),
                                0.0,
                            ));
                            object3d_draw_list.push((1, slot_i));
                        }
                        Object3dKind::YakuTablet {
                            label,
                            active,
                            hover,
                            ..
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
                                    &self.device,
                                    &self.queue,
                                    &self.lit_mesh_material_layout,
                                    &self.tile_sampler,
                                    &self.lit_mesh_relief_default_view,
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
                            object3d_draw_list.push((2, slot_i));
                        }
                        Object3dKind::WoodTablet { label, .. } => {
                            let slot_i = obj3d_wood_slot;
                            obj3d_wood_slot += 1;
                            if slot_i >= MAX_WOOD_TABLET_SLOTS {
                                continue;
                            }
                            let wood_name = match slot_i {
                                0 => "gameplay.action_bar.tablet_sort_suit",
                                1 => "gameplay.action_bar.tablet_sort_rank",
                                2 => "gameplay.action_bar.tablet_cash_in",
                                3 => "gameplay.action_bar.tablet_journal",
                                _ => "gameplay.action_bar.tablet",
                            };
                            let model = self.apply_arrange_override(wood_name, model);
                            let label_hash = tablet_label_hash(label, 512, 192);
                            let inst = &mut self.wood_tablet_instances[slot_i];
                            if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                                let rgba = crate::render::decal::rasterize_wood_tablet_decal(
                                    label,
                                    self.ui_font.as_ref(),
                                );
                                inst.set_decal(
                                    &self.device,
                                    &self.queue,
                                    &self.lit_mesh_material_layout,
                                    &self.tile_sampler,
                                    &self.lit_mesh_relief_default_view,
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
                                wood_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((3, slot_i));
                        }
                        Object3dKind::Relic { relic_id, glow } => {
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
                            let g = glow.clamp(0.0, 1.0);
                            let base_color = if g > 0.0 {
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
                            let material = relic_material_params(*relic_id, base_color, g);
                            self.relic_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            let want_tex = if self.relic_textures.contains_key(relic_id) {
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
                            self.last_relic_models.push(model);
                            self.proj
                                .relic_rects
                                .push(project_unit_cube_rect(model));
                            self.last_debug_pickables.push((
                                relic_arr_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((4, slot_i));
                        }
                        Object3dKind::Pack { kind, pick_id } => {
                            if obj3d_pack_slot >= self.pack_instances.len() {
                                continue;
                            }
                            let slot_i = obj3d_pack_slot;
                            obj3d_pack_slot += 1;
                            // Packs are shop-only; all slots share one placement.
                            let _ = slot_i;
                            let pack_arr_name = "shop.for_sale.packs";
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
                            object3d_draw_list.push((5, slot_i));
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
                            object3d_draw_list.push((7, slot_i));
                        }
                        Object3dKind::ZodiacRibbon { kind } => {
                            // extents: [width, length, depth].
                            let eff_w = obj.extents[0];
                            let eff_l = obj.extents[1];
                            let depth = obj.extents[2];
                            // Push the overall ribbon AABB for arrange-mode picking.
                            // (Individual segments aren't separately selectable.)
                            let ribbon_arr_name = "shop.for_sale.ribbons";
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
                                object3d_draw_list.push((6, slot_i));
                            }
                        }
                        Object3dKind::Coin => {
                            if obj3d_coin_slot >= MAX_COIN_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_coin_slot;
                            obj3d_coin_slot += 1;
                            // Coins are decorative piles in shop (coin_dish) and
                            // gameplay (coin_pile). One placement per scene.
                            let _ = slot_i;
                            let coin_name = match self.active_scene_key {
                                Some("shop") => "shop.shelf.coin_dish".to_string(),
                                Some("gameplay") => "gameplay.score_panel.coin_pile".to_string(),
                                _ => "coin".to_string(),
                            };
                            // extents: [diameter, thickness, diameter]; rotation has yaw baked in.
                            let coin_model = self.apply_arrange_override(
                                &coin_name,
                                translate_rot_scale(
                                    center,
                                    obj.rotation * mesh_y_thickness_along_local_y_to_z_up(),
                                    glam::Vec3::from(obj.extents),
                                ),
                            );
                            let material = MaterialParams {
                                kind: MaterialKind::Metal,
                                base_color: obj.color,
                                specular_strength: 1.0,
                                specular_power: 96.0,
                            };
                            self.coin_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                coin_model,
                                material,
                            );
                            self.last_debug_pickables.push((
                                coin_name,
                                coin_model,
                                glam::Vec3::new(0.5, 0.05, 0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((8, slot_i));
                        }
                        Object3dKind::GoldBar => {
                            if obj3d_bar_slot >= MAX_BAR_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_bar_slot;
                            obj3d_bar_slot += 1;
                            // Gold bars are decorative piles in shop (coin_dish)
                            // and gameplay (coin_pile); route to the scene's pile.
                            let _ = slot_i;
                            let bar_name = match self.active_scene_key {
                                Some("shop") => "shop.shelf.coin_dish".to_string(),
                                Some("gameplay") => "gameplay.score_panel.coin_pile".to_string(),
                                _ => "gold_bar".to_string(),
                            };
                            let model = self.apply_arrange_override(&bar_name, model);
                            let material = MaterialParams {
                                kind: MaterialKind::Metal,
                                base_color: obj.color,
                                specular_strength: 1.0,
                                specular_power: 96.0,
                            };
                            self.bar_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            self.last_debug_pickables.push((
                                bar_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((9, slot_i));
                        }
                        Object3dKind::Book { pick_id } => {
                            // Book is a singleton — shop only.
                            let model = self.apply_arrange_override("shop.props.book", model);
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: obj.color,
                                specular_strength: 0.25,
                                specular_power: 16.0,
                            };
                            self.book_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            let pid = pick_id.unwrap_or(0);
                            self.last_book_model = Some((model, pid));
                            self.last_debug_pickables.push((
                                "shop.props.book".to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((10, 0));
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
                            object3d_draw_list.push((11, slot_i));
                        }
                        Object3dKind::ShopActionProp {
                            label,
                            pick_id,
                            disabled,
                        } => {
                            let slot_i = obj3d_shop_action_prop_slot;
                            obj3d_shop_action_prop_slot += 1;
                            if slot_i >= self.shop_action_prop_instances.len() {
                                continue;
                            }
                            let arrange_name = if slot_i == 0 {
                                "shop.props.reroll_prop"
                            } else {
                                "shop.props.leave_prop"
                            };
                            let model = self.apply_arrange_override(arrange_name, model);
                            let alpha = if *disabled { 0.45 } else { obj.color[3] };
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: [obj.color[0], obj.color[1], obj.color[2], alpha],
                                specular_strength: 0.4,
                                specular_power: 32.0,
                            };
                            let has_label = !label.is_empty();
                            if has_label {
                                let label_hash = tablet_label_hash(label, 512, 192);
                                let inst = &mut self.shop_action_prop_instances[slot_i];
                                if inst.decal_texture.is_none()
                                    || inst.decal_label_hash != label_hash
                                {
                                    let rgba = crate::render::decal::rasterize_wood_tablet_decal(
                                        label,
                                        self.ui_font.as_ref(),
                                    );
                                    inst.set_decal(
                                        &self.device,
                                        &self.queue,
                                        &self.lit_mesh_material_layout,
                                        &self.tile_sampler,
                                        &self.lit_mesh_relief_default_view,
                                        &rgba,
                                        512,
                                        192,
                                    );
                                    inst.decal_label_hash = label_hash;
                                }
                            }
                            self.shop_action_prop_instances[slot_i].write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                                has_label,
                            );
                            if let Some(pid) = pick_id {
                                match slot_i {
                                    0 => self.last_reroll_prop_model = Some((model, *pid)),
                                    1 => self.last_leave_prop_model = Some((model, *pid)),
                                    _ => {}
                                }
                            }
                            self.last_debug_pickables.push((
                                arrange_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((12, slot_i));
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
                                        &self.device,
                                        &self.queue,
                                        &self.lit_mesh_material_layout,
                                        &self.tile_sampler,
                                        &self.lit_mesh_relief_default_view,
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
                                let (scale, rot, trans) =
                                    model.to_scale_rotation_translation();
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
                                // the shallow plate.
                                let local_floor = glam::Vec3::new(0.0, 0.55, 0.0);
                                let world_floor =
                                    trans + rot * (local_floor * scale);
                                // Yaw the card 100° around world +Z so the
                                // crease faces the camera at a slight angle.
                                let yaw =
                                    glam::Quat::from_rotation_z(100.0_f32.to_radians());
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
                            object3d_draw_list.push((13, 0));
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
                            object3d_draw_list.push((14, 0));
                            // Bulb — Glass material. Keep base color close to the
                            // default (warm amber); a modest brightness lift from
                            // the glow envelope so it reads as an active filament
                            // without triggering runaway bloom.
                            let g = glow.clamp(0.0, 1.0);
                            let dm = &self.lamp_bulb_mesh.default_material;
                            let bulb_mat = MaterialParams {
                                kind: crate::render::lit_mesh::MaterialKind::Glass,
                                base_color: [
                                    (dm.base_color[0] * (1.0 + g * 0.25)).min(1.4),
                                    (dm.base_color[1] * (1.0 + g * 0.20)).min(1.3),
                                    (dm.base_color[2] * (1.0 + g * 0.10)).min(1.1),
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
                            object3d_draw_list.push((15, 0));
                            self.last_debug_pickables.push((
                                "shop.props.lamp".to_string(),
                                lamp_model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                        }
                        Object3dKind::Bug { slot } => {
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
                            object3d_draw_list.push((16, slot));
                            self.bug_wing_instances[slot].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                self.bug_wing_mesh.default_material,
                            );
                            object3d_draw_list.push((17, slot));
                        }
                        Object3dKind::BugGhost { slot, alpha } => {
                            let slot = *slot;
                            if slot >= MAX_BUG_GHOST_SLOTS {
                                continue;
                            }
                            let bug_model = translate_rot_scale(
                                center,
                                obj.rotation,
                                glam::Vec3::from(obj.extents),
                            );
                            let alpha = alpha.clamp(0.0, 1.0);
                            let body_mat = self.bug_body_mesh.default_material;
                            let body_tinted = [
                                body_mat.base_color[0],
                                body_mat.base_color[1],
                                body_mat.base_color[2],
                                body_mat.base_color[3] * alpha,
                            ];
                            self.bug_ghost_body_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                body_mat,
                                body_tinted,
                            );
                            object3d_draw_list.push((18, slot));
                            let wing_mat = self.bug_wing_mesh.default_material;
                            let wing_tinted = [
                                wing_mat.base_color[0],
                                wing_mat.base_color[1],
                                wing_mat.base_color[2],
                                wing_mat.base_color[3] * alpha,
                            ];
                            self.bug_ghost_wing_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                wing_mat,
                                wing_tinted,
                            );
                            object3d_draw_list.push((19, slot));
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
                            object3d_draw_list.push((20, slot_i));
                        }
                        // Remaining kinds are stubs until migration is complete.
                        _ => {}
                    }
                }

                let batch_end = object3d_draw_list.len();
                // Patch the placeholder RenderOp that was pushed during the cmd walk.
                // Find the correct Object3dBatch op by scanning from op_batch_idx.
                while op_batch_idx < ops.len() {
                    if let RenderOp::Object3dBatch { start, end } = &mut ops[op_batch_idx] {
                        if *start == 0 && *end == 0 {
                            *start = batch_start;
                            *end = batch_end;
                            op_batch_idx += 1;
                            break;
                        }
                    }
                    op_batch_idx += 1;
                }
                obj3d_cmd_idx += 1;
            }
            let _ = obj3d_cmd_idx;
        }

        // Discard bowl (single instance per cmd; gameplay uses 1).
        // Uses anim_id=1 in the shared hover-state table (matches the legacy
        // convention; will be dropped once gameplay.rs migrates to Object3d).
        let bowl_anim = bowl_cmds
            .first()
            .map(|b| b.hover.clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let bowl_anim = {
            let k = 1.0 - (-14.0 * self.frame_dt).exp();
            let e = self.obj3d_hover_state.entry(1).or_insert(0.0);
            *e += (bowl_anim - *e) * k;
            *e
        };
        for (slot_i, b) in bowl_cmds.iter().enumerate() {
            if slot_i >= MAX_BOWL_SLOTS {
                break;
            }
            let anim = bowl_anim;
            let lift = anim * b.extents[1] * 0.15;
            // Tilt the bowl so its top edge dips toward the camera. The
            // camera looks down at the table from `(0, +Y, +Z)` (~28°
            // below horizontal — see the plaque tilt comment above), so
            // a positive Rx rotation pivots the bowl's +Y axis toward
            // +Z, presenting more of its mouth to the player.
            let tilt = anim * 18.0_f32.to_radians();
            let center = pixel_to_world(
                w,
                h,
                b.world_pos[0],
                b.world_pos[1],
                b.world_pos[2] + b.extents[1] * 0.5 + lift,
            );
            let model = translate_rot_scale(
                center,
                glam::Mat4::from_rotation_x(tilt + b.rotation_x_deg.to_radians()),
                glam::Vec3::new(b.extents[0], b.extents[1], b.extents[2]),
            );
            let model = self.apply_arrange_override("gameplay.action_bar.bowl", model);
            self.bowl_instances[slot_i].write_uniform(
                &self.queue,
                view_proj_arr,
                model,
                self.bowl_mesh.default_material,
            );
            if slot_i == 0 {
                self.proj.bowl_rect = Some(project_aabb_rect(
                    model,
                    BOWL_LOCAL_HALF,
                    BOWL_LOCAL_CENTER_Y,
                ));
                self.last_bowl_model = Some(model);
            }
            self.last_debug_pickables.push((
                "gameplay.action_bar.bowl".to_string(),
                model,
                glam::Vec3::new(BOWL_LOCAL_HALF[0], BOWL_LOCAL_HALF[1], BOWL_LOCAL_HALF[2]),
                BOWL_LOCAL_CENTER_Y,
            ));
        }

        // Bronze mirror (single instance per cmd; gameplay uses 1).
        // Uses anim_id=2 in the shared hover-state table.
        let mirror_anim = mirror_cmds
            .first()
            .map(|m| m.hover.clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let mirror_anim = {
            let k = 1.0 - (-14.0 * self.frame_dt).exp();
            let e = self.obj3d_hover_state.entry(2).or_insert(0.0);
            *e += (mirror_anim - *e) * k;
            *e
        };
        for (slot_i, m) in mirror_cmds.iter().enumerate() {
            if slot_i >= MAX_MIRROR_SLOTS {
                break;
            }
            let anim = mirror_anim;
            let lift = anim * m.extents[1] * 0.15;
            // Tilt the polished face toward the camera so the cast
            // four-spirit relief catches more candle light at hover.
            // Same Rx sign rationale as the bowl above.
            let tilt_deg = m.rotation_x_deg + anim * 22.0;
            let center = pixel_to_world(
                w,
                h,
                m.world_pos[0],
                m.world_pos[1],
                m.world_pos[2] + m.extents[1] * 0.5 + lift,
            );
            let model = translate_rot_scale(
                center,
                rot_rz_rx_deg(tilt_deg, m.rotation_z_deg),
                glam::Vec3::new(m.extents[0], m.extents[1], m.extents[2]),
            );
            let model = self.apply_arrange_override("gameplay.action_bar.mirror", model);
            self.mirror_instances[slot_i].write_uniform(
                &self.queue,
                view_proj_arr,
                model,
                self.mirror_mesh.default_material,
            );
            if slot_i == 0 {
                self.proj.mirror_rect = Some(project_aabb_rect(
                    model,
                    MIRROR_LOCAL_HALF,
                    MIRROR_LOCAL_CENTER_Y,
                ));
                self.last_mirror_model = Some(model);
            }
            self.last_debug_pickables.push((
                "gameplay.action_bar.mirror".to_string(),
                model,
                glam::Vec3::new(
                    MIRROR_LOCAL_HALF[0],
                    MIRROR_LOCAL_HALF[1],
                    MIRROR_LOCAL_HALF[2],
                ),
                MIRROR_LOCAL_CENTER_Y,
            ));
        }

        // Tally fans: upright bone-stick counters in front of the mirror
        // (draws) and river (discards). Each fan emits `count` base-stick
        // instances plus `count` tinted-tip instances, interleaved so slot
        // `2k` is a base and `2k+1` is its tip cap. Sticks keep their
        // angular slots as the count drops — the fan thins from the
        // outermost stick inward, so the upright core stays intact.
        self.proj.peg_rects = [None, None];
        let mut tally_slot_cursor: usize = 0;
        for (fan_i, p) in tally_fan_cmds.iter().enumerate() {
            if fan_i >= MAX_TALLY_FAN_SLOTS {
                break;
            }
            let max_count = p.max_count.max(1);
            let count = p.count.min(max_count) as usize;
            let spread_rad = p.spread_deg.to_radians();
            // Angular slot layout: `max_count` sticks span `spread_rad`
            // symmetrically about 0. For `max_count == 1` the lone slot
            // sits at 0 (upright). Otherwise slot k sits at
            // `-spread/2 + k*(spread/(max_count-1))`.
            let slot_angle = |k: u32| -> f32 {
                if max_count <= 1 {
                    0.0
                } else {
                    -spread_rad * 0.5 + (k as f32) * (spread_rad / (max_count as f32 - 1.0))
                }
            };

            // Fan pivot in world space (at the narrow-base of each stick).
            let pivot = pixel_to_world(w, h, p.world_pos[0], p.world_pos[1], p.world_pos[2]);
            // Orient the stick so its local +Y (length) aligns with world +Z
            // (up off the felt). Same convention used by the other Y-thick
            // procedural meshes.
            let base_orient = mesh_y_thickness_along_local_y_to_z_up();
            // Yaw the whole fan about world +Z (table normal) so scenes can
            // angle the fan plane toward the camera.
            let fan_yaw = Mat4::from_rotation_z(p.rotation_y_deg.to_radians());

            let base_scale = glam::Vec3::new(p.stick_wide, p.stick_len, p.stick_thickness);
            let base_material = self.tally_stick_base_mesh.default_material;
            let tip_material = MaterialParams {
                kind: MaterialKind::Plain,
                base_color: p.tip_color,
                specular_strength: 0.40,
                specular_power: 42.0,
            };

            // Choose the arrange-override name by fan kind so each counter
            // gets its own tunable knob in the debug menu.
            let arrange_name = match p.kind {
                TallyFanKind::Draws => "gameplay.counter.draws_fan",
                TallyFanKind::Discards => "gameplay.counter.discards_fan",
            };

            // Consume sticks from the outermost slot inward so the upright
            // core is the last to disappear. `max_count == count` → all
            // slots visible; `count < max_count` → the outermost
            // `max_count - count` slots are empty. We alternate which side
            // we thin from so the fan stays roughly symmetric.
            let missing = (max_count as usize).saturating_sub(count);
            let mut visible_slots: Vec<u32> = (0..max_count).collect();
            // Thin alternately from right edge, then left edge, then
            // second-right, etc., so the remaining fan stays roughly
            // symmetric even at half-count.
            for trim in 0..missing {
                if trim % 2 == 0 {
                    visible_slots.pop();
                } else {
                    visible_slots.remove(0);
                }
            }

            for (stick_i, &k) in visible_slots.iter().enumerate() {
                if tally_slot_cursor + 1 >= MAX_TALLY_STICK_SLOTS * 2 {
                    break;
                }
                let angle = slot_angle(k);
                // Compose the stick's rotation as
                //   yaw · tilt · orient
                // where `orient` stands the stick up (local +Y → world +Z),
                // `tilt` rotates about world +Y (the axis perpendicular to
                // the default fan plane, so sticks fan in the X-Z plane),
                // and `yaw` then rotates the whole fan about world +Z.
                // Because we translate to `pivot` afterward, the rotation
                // is anchored at the narrow base of every stick.
                let rot = fan_yaw * Mat4::from_rotation_y(angle) * base_orient;
                let model = translate_rot_scale(pivot, rot, base_scale);
                let model = self.apply_arrange_override(arrange_name, model);
                // Only register the first fan's debug pickable to keep the
                // overlay readable.
                if stick_i == 0 {
                    self.last_debug_pickables.push((
                        arrange_name.to_string(),
                        model,
                        glam::Vec3::new(0.5, 0.5, 0.5),
                        0.0,
                    ));
                }
                // Base slot (even), tip slot (odd).
                self.tally_stick_instances[tally_slot_cursor].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    base_material,
                );
                self.tally_stick_instances[tally_slot_cursor + 1].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    tip_material,
                );
                tally_slot_cursor += 2;
            }

            // Project a screen-space bounding rect for the fan so the focus
            // model can find it. Approximate the fan as a unit cube scaled
            // to cover the full angular spread at tip radius — coarse but
            // good enough for hit-testing. The fan stands up in +Z with
            // width along ±X (after yaw) and thickness along ±Y.
            let fan_width = p.stick_len * (spread_rad * 0.5).sin() * 2.0 + p.stick_wide;
            let fan_height = p.stick_len + p.stick_wide * 0.5;
            let fan_center = pixel_to_world(
                w,
                h,
                p.world_pos[0],
                p.world_pos[1],
                p.world_pos[2] + p.stick_len * 0.5,
            );
            let fan_model = translate_rot_scale(
                fan_center,
                fan_yaw,
                glam::Vec3::new(fan_width, p.stick_thickness * 2.0, fan_height),
            );
            let slot = match p.kind {
                TallyFanKind::Draws => 0,
                TallyFanKind::Discards => 1,
            };
            self.proj.peg_rects[slot] = Some(project_unit_cube_rect(fan_model));
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

        // Cascade scoring tokens. Reuses the bone tablet mesh; each token
        // is scaled by `1 + 0.18 * pulse` so the active axis pops on each
        // scoring step. Tint encodes the axis (chips = cool indigo, mult =
        // warm crimson) so the player reads which side just fired.
        let mut cascade_token_slot_cursor: usize = 0;
        for batch in &cascade_token_batches {
            for t in batch.iter() {
                if cascade_token_slot_cursor >= MAX_CASCADE_TOKEN_SLOTS {
                    break;
                }
                let slot_i = cascade_token_slot_cursor;
                cascade_token_slot_cursor += 1;
                let pulse_scale = 1.0 + 0.18 * t.pulse.clamp(0.0, 1.0);
                let center = pixel_to_world(w, h, t.world_pos[0], t.world_pos[1], t.world_pos[2]);
                // Cascade tokens aren't arrangeable; legacy names for the
                // hit-test overlay only.
                let cascade_token_name = match t.kind {
                    CascadeTokenKind::Chips => "gameplay.cascade_token.chips",
                    CascadeTokenKind::Mult => "gameplay.cascade_token.mult",
                };
                let model = translate_rot_scale(
                    center,
                    Mat4::IDENTITY,
                    glam::Vec3::new(
                        t.extents[0] * pulse_scale,
                        t.extents[1] * pulse_scale,
                        t.extents[2] * pulse_scale,
                    ),
                );
                let model = self.apply_arrange_override(cascade_token_name, model);
                let base = match t.kind {
                    CascadeTokenKind::Chips => [0.34, 0.46, 0.78, 1.0],
                    CascadeTokenKind::Mult => [0.85, 0.32, 0.42, 1.0],
                };
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: base,
                    specular_strength: 0.40 + 0.30 * t.pulse.clamp(0.0, 1.0),
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
            }
        }

        // Falling scoring bones — physical objects spawned by the gameplay
        // scene as each cascade step reveals. Same bone-tablet geometry as
        // the cascade tokens, but each instance carries a full 3D pose
        // (gravity-driven world_y + euler tumble) and an alpha that ramps
        // out as a landed bone bleeds its rest timer.
        let mut falling_bone_slot_cursor: usize = 0;
        for batch in &falling_bone_batches {
            for b in batch.iter() {
                if falling_bone_slot_cursor >= MAX_FALLING_BONE_SLOTS {
                    break;
                }
                let slot_i = falling_bone_slot_cursor;
                falling_bone_slot_cursor += 1;
                let center = pixel_to_world(w, h, b.world_pos[0], b.world_pos[1], b.world_pos[2]);
                let model = translate_rot_scale(
                    center,
                    rot_ry_rx_rz_rad(b.rotation[0], b.rotation[1], b.rotation[2]),
                    glam::Vec3::new(b.extents[0], b.extents[1], b.extents[2]),
                );
                let model = self.apply_arrange_override("gameplay.falling_bone", model);
                let base = match b.kind {
                    CascadeTokenKind::Chips => [0.34, 0.46, 0.78, b.alpha],
                    CascadeTokenKind::Mult => [0.85, 0.32, 0.42, b.alpha],
                };
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: base,
                    specular_strength: 0.45,
                    specular_power: 48.0,
                };
                self.falling_bone_instances[slot_i].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    material,
                );
                self.last_debug_pickables.push((
                    "gameplay.falling_bone".to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
            }
        }

        // Floating extruded-glyph score popups. The mesh itself is per-label
        // (each unique string lives in `extruded_glyph_meshes`); only the
        // model matrix and tint vary per popup. We lay each glyph flat-up on
        // the table plane (`R_x(-π/2)` so the +Z extrusion axis points to
        // world +Y) and rotate it slightly toward the camera for legibility.
        let mut extruded_glyph_slot_cursor: usize = 0;
        for batch in &extruded_glyph_batches {
            for g in batch.iter() {
                if extruded_glyph_slot_cursor >= MAX_EXTRUDED_GLYPH_SLOTS {
                    break;
                }
                let slot_i = extruded_glyph_slot_cursor;
                extruded_glyph_slot_cursor += 1;
                let center = pixel_to_world(w, h, g.world_pos[0], g.world_pos[1], g.world_pos[2]);
                let model = translate_rot_scale(
                    center,
                    score_popup_glyph_rot_rad(
                        g.rotation_y,
                        -std::f32::consts::FRAC_PI_2 + g.rotation_x,
                    ),
                    glam::Vec3::splat(g.scale),
                );
                let model = self.apply_arrange_override("gameplay.score_popup", model);
                let material = match g.material {
                    crate::render::draw_cmd::GlyphMaterial::Metal => MaterialParams {
                        kind: MaterialKind::Metal,
                        base_color: g.color,
                        specular_strength: 1.0,
                        specular_power: 128.0,
                    },
                    crate::render::draw_cmd::GlyphMaterial::Polychrome => MaterialParams {
                        kind: MaterialKind::Polychrome,
                        base_color: g.color,
                        specular_strength: 0.85,
                        specular_power: 48.0,
                    },
                    crate::render::draw_cmd::GlyphMaterial::Plain => MaterialParams {
                        kind: MaterialKind::Plain,
                        base_color: g.color,
                        specular_strength: 0.35 + 0.20 * g.emissive.clamp(0.0, 1.0),
                        specular_power: 96.0,
                    },
                };
                self.extruded_glyph_instances[slot_i].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    material,
                );
                self.last_debug_pickables.push((
                    "gameplay.score_popup".to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
            }
        }

        // ── Arrange-mode bounding box overlay ──────────────────────────────
        // When an object is selected in arrange mode, draw a 2D screen-space
        // rectangle outline around its projected AABB so the user can see
        // exactly what they're moving.
        if let Some(ref ov) = self.debug_arrange_override {
            if let Some((_name, model, half, center_y)) = self
                .last_debug_pickables
                .iter()
                .find(|(n, _, _, _)| n == &ov.name)
                .map(|(n, m, h, o)| (n.clone(), *m, *h, *o))
            {
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

            // Cursor → table-plane impulse trail. Unproject the screen
            // cursor, intersect z=5, then interpolate between the previous
            // and current world positions to inject a *chain* of small
            // puffs so the trail has no gaps even at low frame rates or
            // fast flicks.
            if let Some((cx, cy)) = frame.cursor_pos {
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
                            if jump.is_finite() && jump <= max_jump {
                                let speed_threshold = 0.4 * win_scale;
                                if jump > speed_threshold {
                                    // The fluid grid at Standard quality has
                                    // cells ~30 world units wide. Puff radius
                                    // must span several cells so the ray-
                                    // marcher accumulates enough absorption
                                    // across multiple steps to actually see
                                    // the smoke — cursor puffs are transient
                                    // and need to be visible from a single
                                    // injection.
                                    let puff_radius = 32.0 * win_scale;

                                    // Spacing between trail puffs — roughly
                                    // one puff-radius apart so they overlap
                                    // into a continuous ribbon.
                                    let step_size = puff_radius * 0.9;
                                    let n_puffs = ((jump / step_size).ceil() as u32).clamp(1, 8);

                                    for i in 0..n_puffs {
                                        let frac = if n_puffs == 1 {
                                            1.0
                                        } else {
                                            (i as f32 + 1.0) / n_puffs as f32
                                        };
                                        let pos = prev + raw_delta * frac;
                                        let taper = 0.5 + 0.5 * frac;
                                        let puff_density = 0.10 * taper;
                                        fluid.inject_impulse(
                                            pos + glam::Vec3::new(0.0, 0.0, 2.0 * win_scale),
                                            glam::Vec3::ZERO,
                                            puff_radius,
                                            puff_density,
                                            0.08,
                                            i as f32 * 0.37,
                                        );
                                    }
                                }
                            }
                        }
                        self.prev_cursor_world = Some(hit);
                    }
                }
            }

            // Build/rebuild the volume render bind group on first use and
            // after every depth-texture recreation (resize). The smoke pass
            // samples a SNAPSHOT of the depth (`depth_copy_view`) that we
            // copy between the pre-smoke and post-smoke render passes — the
            // live `depth_view` would alias the active depth attachment.
            if self.fluid_render_bg_dirty {
                fluid.rebuild_render_bind_group(
                    &self.device,
                    &self.depth_copy_view,
                    &self.point_lights_buffer,
                );
                self.fluid_render_bg_dirty = false;
            }

            // (Re)allocate the offscreen smoke target whenever the user
            // changes the detail dropdown OR the window resizes. Cheap
            // no-op when nothing changed.
            fluid.set_detail(&self.device, smoke_detail, &self.depth_copy_view);

            // Upload the per-frame camera uniform consumed by the volume
            // raymarch shader.
            fluid.upload_camera_uniform(
                &self.queue,
                view_proj,
                cam_pos,
                smoke_intensity,
                smoke_sim_quality,
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
        for batch in &candle_batches {
            for (slot_i, placement) in batch.iter().enumerate() {
                let Some(instances) = self.candle_instances.get(slot_i) else {
                    break;
                };
                let base = pixel_to_world(
                    w,
                    h,
                    placement.world_pos[0],
                    placement.world_pos[1],
                    placement.world_pos[2],
                );
                let s = placement.scale;
                let model = translate_rot_scale(
                    base,
                    mesh_y_thickness_along_local_y_to_z_up(),
                    glam::Vec3::new(s, s * placement.height_scale, s),
                );
                instances[0].write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                instances[1].write_shadow_uniform(&self.queue, light_view_proj_arr, model);
            }
        }
        {
            let mut relic_shadow_cursor: usize = 0;
            for batch in &relic_batches {
                for p in batch.iter() {
                    if relic_shadow_cursor >= MAX_RELIC_SLOTS {
                        break;
                    }
                    let slot_i = relic_shadow_cursor;
                    relic_shadow_cursor += 1;
                    let center = pixel_to_world(
                        w,
                        h,
                        p.world_pos[0],
                        p.world_pos[1],
                        p.world_pos[2] + p.half_extents[1],
                    );
                    let rotation = rot_rx_rz_deg(p.rotation_x_deg, p.rotation_z_deg);
                    let model = translate_rot_scale(
                        center,
                        rotation,
                        glam::Vec3::new(
                            p.half_extents[0] * 2.0,
                            p.half_extents[1] * 2.0,
                            p.half_extents[2] * 2.0,
                        ),
                    );
                    self.relic_instances[slot_i].write_shadow_uniform(
                        &self.queue,
                        light_view_proj_arr,
                        model,
                    );
                }
            }
        }
        // Curio cabinet shadow caster (single instance).
        if let Some(c) = cabinet_cmds.first() {
            let center = pixel_to_world(w, h, c.center_pos[0], c.center_pos[1], c.center_pos[2]);
            let model = translate_rot_scale(
                center,
                Mat4::IDENTITY,
                glam::Vec3::new(c.extents[0], c.extents[1], c.extents[2]),
            );
            self.cabinet_instance
                .write_shadow_uniform(&self.queue, light_view_proj_arr, model);
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
        // Auxiliary dish shadow casters.
        for (slot_i, d) in aux_dish_cmds.iter().enumerate() {
            let center = pixel_to_world(
                w,
                h,
                d.center_pos[0],
                d.center_pos[1],
                d.center_pos[2] + d.extents[1] * 0.5,
            );
            let oriented = if d.round {
                mesh_y_thickness_along_local_y_to_z_up()
            } else {
                Mat4::IDENTITY
            };
            let model = translate_rot_scale(
                center,
                oriented,
                glam::Vec3::new(d.extents[0], d.extents[1], d.extents[2]),
            );
            self.aux_dish_instances[slot_i].write_shadow_uniform(
                &self.queue,
                light_view_proj_arr,
                model,
            );
        }
        // Ribbon shadow casters — mirrors the 3-slot-per-ribbon logic from
        // the main pass so shadow silhouettes match the visible geometry.
        {
            let mut ribbon_shadow_cursor: usize = 0;
            for batch in &ribbon_batches {
                for r in batch.iter() {
                    if ribbon_shadow_cursor >= MAX_RIBBON_SLOTS {
                        break;
                    }
                    let anchor =
                        pixel_to_world(w, h, r.anchor_pos[0], r.anchor_pos[1], r.anchor_pos[2]);
                    let eff_w = r.width;
                    let eff_l = r.length;
                    let depth = eff_w * 0.15;
                    let base_transform = translate_rot_scale(
                        anchor,
                        rot_rz_ry_rx_deg(r.rotation_x_deg, r.rotation_y_deg, r.rotation_z_deg),
                        glam::Vec3::splat(1.0),
                    );

                    if r.kind.is_some() {
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
                        // Top cap
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
                        // Middle
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
                        // Bottom cap
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
        // Talisman shadow casters.
        {
            let mut talisman_shadow_cursor: usize = 0;
            for batch in &talisman_batches {
                for t in batch.iter() {
                    if talisman_shadow_cursor >= MAX_TALISMAN_SLOTS {
                        break;
                    }
                    let slot_i = talisman_shadow_cursor;
                    talisman_shadow_cursor += 1;
                    let center =
                        pixel_to_world(w, h, t.center_pos[0], t.center_pos[1], t.center_pos[2]);
                    let sx = t.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                    let sy = t.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                    let sz = t.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                    let model = translate_rot_scale(
                        center,
                        rot_rz_ry_rx_deg(t.rotation_x_deg, t.rotation_y_deg, t.rotation_z_deg),
                        glam::Vec3::new(sx, sy, sz),
                    );
                    self.talisman_instances[slot_i].write_shadow_uniform(
                        &self.queue,
                        light_view_proj_arr,
                        model,
                    );
                }
            }
        }
        // Coin shadow casters.
        {
            let mut coin_shadow_cursor: usize = 0;
            for batch in &coin_batches {
                for c in batch.iter() {
                    if coin_shadow_cursor >= MAX_COIN_SLOTS {
                        break;
                    }
                    let slot_i = coin_shadow_cursor;
                    coin_shadow_cursor += 1;
                    let center =
                        pixel_to_world(w, h, c.world_pos[0], c.world_pos[1], c.world_pos[2]);
                    let model = translate_rot_scale(
                        center,
                        rot_z_rad(c.rotation_y) * mesh_y_thickness_along_local_y_to_z_up(),
                        glam::Vec3::new(c.radius * 2.0, c.thickness, c.radius * 2.0),
                    );
                    self.coin_instances[slot_i].write_shadow_uniform(
                        &self.queue,
                        light_view_proj_arr,
                        model,
                    );
                }
            }
        }
        // Gold bar shadow casters.
        {
            let mut bar_shadow_cursor: usize = 0;
            for batch in &bar_batches {
                for b in batch.iter() {
                    if bar_shadow_cursor >= MAX_BAR_SLOTS {
                        break;
                    }
                    let slot_i = bar_shadow_cursor;
                    bar_shadow_cursor += 1;
                    let center =
                        pixel_to_world(w, h, b.world_pos[0], b.world_pos[1], b.world_pos[2]);
                    let model = translate_rot_scale(
                        center,
                        rot_z_rad(b.rotation_y),
                        glam::Vec3::new(
                            b.half_extents[0] * 2.0,
                            b.half_extents[1] * 2.0,
                            b.half_extents[2] * 2.0,
                        ),
                    );
                    self.bar_instances[slot_i].write_shadow_uniform(
                        &self.queue,
                        light_view_proj_arr,
                        model,
                    );
                }
            }
        }
        // Book shadow caster.
        if let Some(b) = book_cmd {
            let center = pixel_to_world(w, h, b.world_pos[0], b.world_pos[1], b.world_pos[2]);
            let model = translate_rot_scale(
                center,
                rot_z_rad(b.rotation_y),
                glam::Vec3::new(
                    b.half_extents[0] * 2.0,
                    b.half_extents[1] * 2.0,
                    b.half_extents[2] * 2.0,
                ),
            );
            self.book_instance
                .write_shadow_uniform(&self.queue, light_view_proj_arr, model);
        }
        if needs_dish {
            if let Some((lo_x, lo_y, hi_x, hi_y)) = dish_bounds {
                let cx = (lo_x + hi_x) * 0.5;
                let cy = (lo_y + hi_y) * 0.5;
                let dw = (hi_x - lo_x).max(40.0);
                let dd = (hi_y - lo_y).max(28.0);
                let dh = 10.0_f32;
                let center = pixel_to_world(w, h, cx, cy, dh * 0.5);
                let model =
                    translate_rot_scale(center, Mat4::IDENTITY, glam::Vec3::new(dw, dh, dd));
                self.dish_instance
                    .write_shadow_uniform(&self.queue, light_view_proj_arr, model);
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
                    let stg = make_showcase_tile_gpu(
                        &self.device,
                        &self.queue,
                        &self.tile_material_layout,
                        &self.shadow_caster_layout,
                        &self.tile_primitives,
                        &self.tile_sampler,
                        self.tile_base_color_factor,
                        self.ui_font.as_ref(),
                        self.emoji_font.as_ref(),
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
                        self.showcase_tiles[slot_cursor] = make_showcase_tile_gpu(
                            &self.device,
                            &self.queue,
                            &self.tile_material_layout,
                            &self.shadow_caster_layout,
                            &self.tile_primitives,
                            &self.tile_sampler,
                            self.tile_base_color_factor,
                            self.ui_font.as_ref(),
                            self.emoji_font.as_ref(),
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
                            if speed > 0.5 {
                                if let Some(ref mut fluid) = self.fluid {
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
                                color: [1.00, 0.78, 0.32, 0.55],
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
                    self.queue.write_buffer(
                        &stg.uniform_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_proj: view_proj_arr,
                            model: model.to_cols_array(),
                            base_color_factor: sc_bcf,
                            cam_pos: cam_pos.to_array(),
                            _pad: 0.0,
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
                                _pad: 0.0,
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
                let any_hand = showcase_tile_batches
                    .iter()
                    .flat_map(|b| b.iter())
                    .any(|p| p.pick_id.is_some());
                any_hand
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
        for model in &self.last_relic_models {
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
            self.proj.talisman_rects.push(project_aabb_rect(
                *model,
                TALISMAN_LOCAL_HALF,
                0.0,
            ));
        }

        // Sync singleton shop-prop models (journal book, reroll prop, leave
        // prop, sell tray) into `aux_dish_rects` so focus nav can reach
        // them. Dishes authored via `DishExplicit` were already pushed
        // during their pass; these props come through Object3d kinds that
        // only update model snapshots, so we project them here. Packs live
        // in `pack_rects` (both the PackBatch and Object3d paths populate
        // it) and get appended last.
        for prop in [
            self.last_book_model,
            self.last_reroll_prop_model,
            self.last_leave_prop_model,
            self.last_sell_tray_model,
        ]
        .iter()
        .flatten()
        {
            let (model, pid) = *prop;
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
            fluid.step(
                &mut encoder,
                &self.queue,
                step_dt,
                smoke_intensity,
                smoke_sim_quality,
            );
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

            // Candles (wax + wick).
            for batch in &candle_batches {
                for (slot_i, _) in batch.iter().enumerate() {
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

            // Dish.
            if needs_dish && dish_bounds.is_some() {
                shadow_pass.set_bind_group(0, &self.dish_instance.shadow_bind_group, &[]);
                shadow_pass.set_vertex_buffer(0, self.dish_mesh.vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(
                    self.dish_mesh.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                shadow_pass.draw_indexed(0..self.dish_mesh.index_count, 0, 0..1);
            }

            // Relic boxes.
            {
                let mut slot_i = 0usize;
                for batch in &relic_batches {
                    for p in batch.iter() {
                        if slot_i >= MAX_RELIC_SLOTS {
                            break;
                        }
                        let Some(inst) = self.relic_instances.get(slot_i) else {
                            break;
                        };
                        let mesh = self.relic_mesh_for(p.relic_id);
                        shadow_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        shadow_pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                        slot_i += 1;
                    }
                }
                for batch in &relic_showcase_batches {
                    for p in batch.iter() {
                        if slot_i >= MAX_RELIC_SLOTS {
                            break;
                        }
                        let Some(inst) = self.relic_instances.get(slot_i) else {
                            break;
                        };
                        let mesh = self.relic_mesh_for(p.relic_id);
                        shadow_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        shadow_pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                        slot_i += 1;
                    }
                }
            }

            // Curio cabinet (shop).
            if !cabinet_cmds.is_empty() {
                shadow_pass.set_vertex_buffer(0, self.cabinet_mesh.vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(
                    self.cabinet_mesh.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                shadow_pass.set_bind_group(0, &self.cabinet_instance.shadow_bind_group, &[]);
                shadow_pass.draw_indexed(0..self.cabinet_mesh.index_count, 0, 0..1);
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

            // Auxiliary dishes (shop). Each dish picks square or round mesh
            // based on its `round` flag; switch the bound vertex/index buffers
            // when the choice changes between consecutive draws.
            {
                let n_aux = aux_dish_cmds.len();
                if n_aux > 0 {
                    let mut bound_round: Option<bool> = None;
                    for slot_i in 0..n_aux {
                        let round = aux_dish_cmds[slot_i].round;
                        if bound_round != Some(round) {
                            let mesh = if round {
                                &self.round_dish_mesh
                            } else {
                                &self.dish_mesh
                            };
                            shadow_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                            shadow_pass.set_index_buffer(
                                mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            bound_round = Some(round);
                        }
                        let mesh = if round {
                            &self.round_dish_mesh
                        } else {
                            &self.dish_mesh
                        };
                        shadow_pass.set_bind_group(
                            0,
                            &self.aux_dish_instances[slot_i].shadow_bind_group,
                            &[],
                        );
                        shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                    }
                }
            }

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

            // Talismans (shop).
            {
                let total_talismans = talisman_batches
                    .iter()
                    .map(|b| b.len())
                    .sum::<usize>()
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

            // Coins (shop).
            {
                let total_coins = coin_batches
                    .iter()
                    .map(|b| b.len())
                    .sum::<usize>()
                    .min(MAX_COIN_SLOTS);
                if total_coins > 0 {
                    shadow_pass.set_vertex_buffer(0, self.coin_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.coin_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..total_coins {
                        let Some(inst) = self.coin_instances.get(slot_i) else {
                            break;
                        };
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..self.coin_mesh.index_count, 0, 0..1);
                    }
                }
            }

            // Gold bars (shop). Uses the same unit-box mesh as relics.
            {
                let total_bars = bar_batches
                    .iter()
                    .map(|b| b.len())
                    .sum::<usize>()
                    .min(MAX_BAR_SLOTS);
                if total_bars > 0 {
                    shadow_pass.set_vertex_buffer(0, self.relic_box_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.relic_box_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..total_bars {
                        let Some(inst) = self.bar_instances.get(slot_i) else {
                            break;
                        };
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..self.relic_box_mesh.index_count, 0, 0..1);
                    }
                }
            }

            // Book shadow (journal bookend).
            if book_cmd.is_some() {
                shadow_pass.set_vertex_buffer(0, self.book_mesh.vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(
                    self.book_mesh.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                shadow_pass.set_bind_group(0, &self.book_instance.shadow_bind_group, &[]);
                shadow_pass.draw_indexed(0..self.book_mesh.index_count, 0, 0..1);
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
                RenderOp::SunlitWater => {
                    pass.set_pipeline(&self.sunlit_water_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                RenderOp::ShootingStarCascade => {
                    pass.set_pipeline(&self.shooting_star_cascade_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
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
                RenderOp::CandleBatch(batch_idx) => {
                    let batch = candle_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    for (slot_i, _) in batch.iter().enumerate() {
                        let Some(instances) = self.candle_instances.get(slot_i) else {
                            break;
                        };
                        // Wax body.
                        pass.set_bind_group(0, &instances[0].bind_group, &[]);
                        pass.set_vertex_buffer(0, self.candle_wax_mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            self.candle_wax_mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..self.candle_wax_mesh.index_count, 0, 0..1);
                        // Wick.
                        pass.set_bind_group(0, &instances[1].bind_group, &[]);
                        pass.set_vertex_buffer(0, self.candle_wick_mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            self.candle_wick_mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..self.candle_wick_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::Dish => {
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(0, &self.dish_instance.bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.dish_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.dish_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..self.dish_mesh.index_count, 0, 0..1);
                }
                RenderOp::RelicBatch(batch_idx) => {
                    let batch = relic_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    // Compute the global slot offset for this batch from
                    // the cumulative lengths of preceding RelicBatch cmds.
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += relic_batches[prev].len();
                    }
                    for (i, p) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.relic_instances.get(slot_i) else {
                            break;
                        };
                        let mesh = self.relic_mesh_for(p.relic_id);
                        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                    }

                    // Activation halos: drawn after the boxes so the
                    // additive bloom adds on top of the lit relic. Uses
                    // the selected-tile glow pipeline since the falloff
                    // is generic — any rect with color.a as intensity.
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
                RenderOp::RelicShowcaseBatch(batch_idx) => {
                    let batch = relic_showcase_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    let mut start_slot = relic_batches.iter().map(|b| b.len()).sum::<usize>();
                    for prev in 0..*batch_idx {
                        start_slot += relic_showcase_batches[prev].len();
                    }
                    for (i, p) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.relic_instances.get(slot_i) else {
                            break;
                        };
                        let mesh = self.relic_mesh_for(p.relic_id);
                        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::PackBatch(batch_idx) => {
                    let batch = pack_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.pack_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.pack_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += pack_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.pack_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.pack_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::DishExplicit(idx) => {
                    if *idx < self.aux_dish_instances.len() {
                        let mesh = if aux_dish_cmds
                            .get(*idx)
                            .map(|d| d.round)
                            .unwrap_or(false)
                        {
                            &self.round_dish_mesh
                        } else {
                            &self.dish_mesh
                        };
                        pass.set_pipeline(&self.lit_mesh_pipeline);
                        pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                        pass.set_bind_group(0, &self.aux_dish_instances[*idx].bind_group, &[]);
                        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::CurioCabinet => {
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(0, &self.cabinet_instance.bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.cabinet_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.cabinet_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..self.cabinet_mesh.index_count, 0, 0..1);
                }
                RenderOp::ShrineBatch(batch_idx) => {
                    let batch = shrine_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.shrine_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.shrine_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += shrine_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.shrine_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.shrine_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::ZodiacBatch(batch_idx) => {
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.ribbon_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.ribbon_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += self
                            .last_ribbon_batch_slot_counts
                            .get(prev)
                            .copied()
                            .unwrap_or(0);
                    }
                    let slot_count = self
                        .last_ribbon_batch_slot_counts
                        .get(*batch_idx)
                        .copied()
                        .unwrap_or(0);
                    for i in 0..slot_count {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.ribbon_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.ribbon_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::TalismanBatch(batch_idx) => {
                    let batch = talisman_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.talisman_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.talisman_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += talisman_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.talisman_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.talisman_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::CoinBatch(batch_idx) => {
                    let batch = coin_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.coin_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.coin_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += coin_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.coin_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.coin_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::GoldBarBatch(batch_idx) => {
                    let batch = bar_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.relic_box_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.relic_box_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += bar_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.bar_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.relic_box_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::Book => {
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(0, &self.book_instance.bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.book_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.book_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..self.book_mesh.index_count, 0, 0..1);
                }
                RenderOp::Plaque(slot_i) => {
                    let Some(inst) = self.plaque_instances.get(*slot_i) else {
                        return;
                    };
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(0, &inst.bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.plaque_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.plaque_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..self.plaque_mesh.index_count, 0, 0..1);
                }
                RenderOp::Ofuda(slot_i) => {
                    let Some(inst) = self.ofuda_instances.get(*slot_i) else {
                        return;
                    };
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(0, &inst.bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.ofuda_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.ofuda_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..self.ofuda_mesh.index_count, 0, 0..1);
                }
                RenderOp::YakuTabletBatch(batch_idx) => {
                    let batch = yaku_tablet_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.bone_tablet_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.bone_tablet_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += yaku_tablet_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.yaku_tablet_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.bone_tablet_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::WoodTabletBatch(batch_idx) => {
                    let batch = wood_tablet_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.wood_tablet_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.wood_tablet_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += wood_tablet_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.wood_tablet_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.wood_tablet_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::Bowl(slot_i) => {
                    let Some(inst) = self.bowl_instances.get(*slot_i) else {
                        return;
                    };
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(0, &inst.bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.bowl_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.bowl_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..self.bowl_mesh.index_count, 0, 0..1);
                }
                RenderOp::Mirror(slot_i) => {
                    let Some(inst) = self.mirror_instances.get(*slot_i) else {
                        return;
                    };
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(0, &inst.bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.mirror_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.mirror_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..self.mirror_mesh.index_count, 0, 0..1);
                }
                RenderOp::TallyFan(fan_i) => {
                    // Each fan owns `count` sticks; each stick owns two
                    // consecutive slots in `tally_stick_instances` (base,
                    // then tint cap). Walk previous fans to find our
                    // starting slot, then draw our sticks as base+tip pairs
                    // with the corresponding meshes.
                    let fan = tally_fan_cmds[*fan_i];
                    let mut start_slot = 0usize;
                    for prev in 0..*fan_i {
                        let pf = tally_fan_cmds[prev];
                        start_slot +=
                            (pf.count.min(pf.max_count.max(1)) as usize).saturating_mul(2);
                    }
                    let n_sticks = fan.count.min(fan.max_count.max(1)) as usize;
                    if n_sticks == 0 {
                        return;
                    }
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);

                    // Bone base segments.
                    pass.set_vertex_buffer(0, self.tally_stick_base_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.tally_stick_base_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for k in 0..n_sticks {
                        let s = start_slot + 2 * k;
                        let Some(inst) = self.tally_stick_instances.get(s) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(
                            0..self.tally_stick_base_mesh.index_count,
                            0,
                            0..1,
                        );
                    }

                    // Tinted tip caps.
                    pass.set_vertex_buffer(0, self.tally_stick_tip_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.tally_stick_tip_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for k in 0..n_sticks {
                        let s = start_slot + 2 * k + 1;
                        let Some(inst) = self.tally_stick_instances.get(s) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.tally_stick_tip_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::WallStack(slot_i) => {
                    let cmd = wall_stack_cmds[*slot_i];
                    let mut start_slot = 0usize;
                    for prev in 0..*slot_i {
                        start_slot +=
                            (wall_stack_cmds[prev].remaining as usize).min(MAX_WALL_TILE_SLOTS);
                    }
                    let n = (cmd.remaining as usize).min(MAX_WALL_TILE_SLOTS);
                    if n == 0 {
                        return;
                    }
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.bone_tablet_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.bone_tablet_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for k in 0..n {
                        let s = start_slot + k;
                        let Some(inst) = self.wall_tile_instances.get(s) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.bone_tablet_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::CascadeTokenBatch(batch_idx) => {
                    let batch = cascade_token_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.bone_tablet_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.bone_tablet_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += cascade_token_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.cascade_token_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.bone_tablet_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::FallingBoneBatch(batch_idx) => {
                    let batch = falling_bone_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.bone_tablet_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.bone_tablet_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += falling_bone_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        if slot_i >= MAX_FALLING_BONE_SLOTS {
                            break;
                        }
                        let Some(inst) = self.falling_bone_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.bone_tablet_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::ExtrudedGlyphBatch(batch_idx) => {
                    // Each placement is a different label string with its own
                    // mesh, so unlike the cascade-token / falling-bone draws
                    // above we re-bind the vertex/index buffers per draw.
                    let batch = extruded_glyph_batches[*batch_idx];
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += extruded_glyph_batches[prev].len();
                    }
                    for (i, p) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        if slot_i >= MAX_EXTRUDED_GLYPH_SLOTS {
                            break;
                        }
                        let Some(inst) = self.extruded_glyph_instances.get(slot_i) else {
                            break;
                        };
                        let Some(mesh) = self.extruded_glyph_meshes.get(&p.label) else {
                            // Mesh failed to build (font missing or empty
                            // string). The build site at DrawCmd::ExtrudedGlyphBatch
                            // already logs the tessellation failure; log here
                            // too so we see the per-frame skip at draw time.
                            log::warn!(
                                "[extruded glyph] no GPU mesh for label {:?} — skipping draw",
                                p.label
                            );
                            continue;
                        };
                        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::Object3dBatch { start, end } => {
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    let mut current_blended = false;
                    for &(kind_id, slot_i) in &object3d_draw_list[*start..*end] {
                        let want_blended = matches!(kind_id, 18 | 19);
                        if want_blended != current_blended {
                            if want_blended {
                                pass.set_pipeline(&self.lit_mesh_blended_pipeline);
                            } else {
                                pass.set_pipeline(&self.lit_mesh_pipeline);
                            }
                            current_blended = want_blended;
                        }
                        // Relic mesh is looked up per relic_id stored in relic_slot_texture.
                        if kind_id == 4 {
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
                        let (mesh, inst_opt): (&LitMeshGpu, Option<&LitMeshInstance>) =
                            match kind_id {
                                0 => (&self.plaque_mesh, self.plaque_instances.get(slot_i)),
                                1 => (&self.ofuda_mesh, self.ofuda_instances.get(slot_i)),
                                2 => (
                                    &self.bone_tablet_mesh,
                                    self.yaku_tablet_instances.get(slot_i),
                                ),
                                3 => (
                                    &self.wood_tablet_mesh,
                                    self.wood_tablet_instances.get(slot_i),
                                ),
                                5 => (&self.pack_mesh, self.pack_instances.get(slot_i)),
                                6 => (&self.ribbon_mesh, self.ribbon_instances.get(slot_i)),
                                7 => (&self.talisman_mesh, self.talisman_instances.get(slot_i)),
                                8 => (&self.coin_mesh, self.coin_instances.get(slot_i)),
                                9 => (&self.relic_box_mesh, self.bar_instances.get(slot_i)),
                                10 => (&self.book_mesh, Some(&self.book_instance)),
                                11 => (&self.shrine_mesh, self.shrine_instances.get(slot_i)),
                                12 => (
                                    &self.shop_action_prop_mesh,
                                    self.shop_action_prop_instances.get(slot_i),
                                ),
                                13 => (&self.round_dish_mesh, Some(&self.sell_tray_instance)),
                                14 => (&self.lamp_body_mesh, Some(&self.lamp_body_instance)),
                                15 => (&self.lamp_bulb_mesh, Some(&self.lamp_bulb_instance)),
                                16 => (&self.bug_body_mesh, self.bug_body_instances.get(slot_i)),
                                17 => (&self.bug_wing_mesh, self.bug_wing_instances.get(slot_i)),
                                18 => (
                                    &self.bug_body_mesh,
                                    self.bug_ghost_body_instances.get(slot_i),
                                ),
                                19 => (
                                    &self.bug_wing_mesh,
                                    self.bug_ghost_wing_instances.get(slot_i),
                                ),
                                20 => (&self.orb_mesh, self.orb_instances.get(slot_i)),
                                _ => continue,
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
                }
                RenderOp::ShowcaseTileBatch(batch_idx) => {
                    if !self.tile_primitives.is_empty() {
                        let batch = showcase_tile_batches[*batch_idx];
                        if !batch.is_empty() {
                            pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                            pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                            let mut start_slot = 0usize;
                            for prev in 0..*batch_idx {
                                start_slot += showcase_tile_batches[prev].len();
                            }

                            // Glow halos for selected hand tiles (additive, drawn before mesh).
                            let has_glow = batch.iter().any(|p| p.glow);
                            if has_glow {
                                if let Some(ref tgb) = tile_glow_buffer {
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
                    if smoke_intensity != crate::persistence::SmokeIntensity::Off {
                        if let Some(ref fluid) = self.fluid {
                            // Composite the offscreen smoke target onto the
                            // swap chain. The actual raymarch ran earlier in
                            // its own offscreen pass; this is just a
                            // bilinear sample + premultiplied blend.
                            fluid.draw_composite(pass);
                        }
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
                RenderOp::FlameBatch { buf_idx, count } => {
                    // When the volumetric smoke sim is active, candle flames
                    // are rendered as 3D emission inside the volume lightbake
                    // pass — skip the legacy 2D additive quads so we don't
                    // double-draw. Fall back to 2D flames when smoke is Off
                    // (the fluid sim doesn't step, so volumetric flames
                    // wouldn't appear).
                    if smoke_intensity == crate::persistence::SmokeIntensity::Off {
                        pass.set_pipeline(&self.flame_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
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
                RenderOp::RelicIconDraw(idx) => {
                    let image = &image_draws[*idx];
                    pass.set_pipeline(&self.image_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &image.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, image.inst_buf.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
            }
        }; // end process_op closure

        // ── Pass A: clear + draw everything that lives behind the smoke ──
        {
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
                timestamp_writes: None,
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
                if matches!(op, RenderOp::TextDraw(_) | RenderOp::Plaque(_)) {
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

        // ── Plaque pass: draw the hanging score plaque (with its engraved
        // text decal) into the swapchain *after* the SSR snapshot above
        // and *before* the smoke pass below. Loading both colour and
        // depth keeps the rest of pass A intact while the plaques write
        // their own depth (so smoke occludes them correctly) and their
        // decal pixels (which never enter `scene_prev` / `ssr_prev_depth`
        // because the snapshot already happened).
        if ops.iter().any(|o| matches!(o, RenderOp::Plaque(_))) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("plaque-pass"),
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
                timestamp_writes: None,
                multiview_mask: None,
            });
            // Walk only the pre-smoke ops slice — plaques live in pass A
            // ordering; any plaque cmd queued after the FluidSmoke marker
            // would still get drawn here, which is fine because smoke
            // already composites over the plaque depth via the
            // `depth_copy_texture` snapshot taken below.
            for op in &ops[..split_end] {
                if matches!(op, RenderOp::Plaque(_)) {
                    process_op(&mut pass, op);
                }
            }
        }

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
            if self.gpu_profiler.is_sampling() {
                if smoke_intensity != crate::persistence::SmokeIntensity::Off {
                    if let Some(ref fluid) = self.fluid {
                        fluid.set_render_mode_encoder(&mut encoder, true);
                        let scissor = fluid.screen_aabb_rect(view_proj);
                        fluid.render_offscreen(
                            &mut encoder,
                            &self.globals_bind_group,
                            scissor,
                            None,
                        );
                        fluid.set_render_mode_encoder(&mut encoder, false);
                    }
                }
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
            if smoke_intensity != crate::persistence::SmokeIntensity::Off {
                if let Some(ref fluid) = self.fluid {
                    let scissor = fluid.screen_aabb_rect(view_proj);
                    fluid.render_offscreen(&mut encoder, &self.globals_bind_group, scissor, None);
                }
            }

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
                timestamp_writes: None,
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
        let composite_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [0.0; 4],
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

        // GPU profiler: resolve query set + stage readback before submit,
        // then block on map after submit so the readback is frame-accurate.
        // Both calls are no-ops when no profiling session is active.
        self.gpu_profiler.before_submit(&mut encoder);
        self.queue.submit(std::iter::once(encoder.finish()));
        surface_frame.present();
        self.gpu_profiler.after_submit(&self.device);
        Ok(())
    }
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
