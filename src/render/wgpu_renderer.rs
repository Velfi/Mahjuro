//! WGPU: depth-tested 3D tile meshes for the hand + 2D UI quads on top.

use std::collections::HashMap;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use glam::Mat4;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::core::relic::RelicId;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::TilePackKind;
use crate::render::bone_tablet_mesh::build_bone_tablet_mesh;
use crate::render::candle_mesh::{CandlePlacement, build_candle_wax_mesh, build_candle_wick_mesh};
use crate::render::coin_mesh::build_coin_mesh;
use crate::render::curio_cabinet_mesh::build_curio_cabinet_mesh;
use crate::render::decal::{
    LabelAlign, load_noto_emoji_font, load_ui_font, rasterize_label_styled_with_fallback,
    rasterize_tile_face_decal, tile_short_label, tile_suit_emoji,
};
use crate::render::dora_stand_mesh::build_dora_stand_mesh;
use crate::render::draw_cmd::{
    BowlPlacement, CascadeTokenKind, CascadeTokenPlacement, CoinPlacement, CurioCabinetPlacement,
    DishExplicit, DoraStandPlacement, DrawCmd, ExtrudedGlyphPlacement, FallingBonePlacement,
    MirrorPlacement, OfudaPlacement, PackPlacement, PegBlockPlacement, PlaquePlacement,
    RelicPlacement, ShowcaseTilePlacement, ShrinePlacement, TalismanPlacement, UiFrame,
    WallStackPlacement, WoodTabletPlacement, YakuTabletPlacement, ZodiacRibbonPlacement,
};
use crate::render::lit_mesh::{
    LitMeshGpu, LitMeshInstance, MaterialKind, MaterialParams, ShadowCasterUniform, ShadowGlobals,
    SsrGlobals, create_lit_mesh_material_layout, create_lit_mesh_ssr_layout,
    create_shadow_caster_layout, create_shadow_sample_layout,
};
use crate::render::mirror_mesh::{MIRROR_LOCAL_CENTER_Y, MIRROR_LOCAL_HALF, build_mirror_mesh};
use crate::render::ofuda_mesh::build_ofuda_mesh;
use crate::render::peg_block_mesh::build_peg_block_mesh;
use crate::render::plaque_mesh::build_plaque_mesh;
use crate::render::relic_dish::{build_dish_mesh, build_unit_box_mesh};
use crate::render::ribbon_mesh::build_ribbon_mesh;
use crate::render::river_mesh::{
    RIVER_LOCAL_CENTER_Y as BOWL_LOCAL_CENTER_Y, RIVER_LOCAL_HALF as BOWL_LOCAL_HALF,
    build_river_mesh,
};
use crate::render::shrine_mesh::build_shrine_mesh;
use crate::render::table_mesh::build_table_mesh;
use crate::render::talisman_mesh::{TALISMAN_LOCAL_HALF, build_talisman_mesh};
use crate::render::tile_glb::{Vertex3dTex, load_glb_tile_from_bytes, normalize_mesh};
use crate::render::wood_tablet_mesh::build_wood_tablet_mesh;
use crate::scenes::BackgroundId;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    screen: [f32; 2],
    time: f32,
    gamma: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [f32; 16],
    model: [f32; 16],
    base_color_factor: [f32; 4],
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
    /// `world_z = y - h/2`). The third position component is treated as the
    /// height above the table plane (`world_y`).
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
            let wx = l.pos[0] - screen_w * 0.5;
            let wz = l.pos[1] - screen_h * 0.5;
            let wy = l.pos[2];
            lights[i] = PointLightGpu {
                pos: [wx, wy, wz, l.radius],
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
    tile_id: (Suit, u8, Option<crate::core::tile::TileEnhancement>),
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

/// Simplified `HandTileGpu` for showcase tiles (pack celebration, etc.).
/// No outline buffers, no text metadata — display only.
struct ShowcaseTileGpu {
    uniform_buffer: wgpu::Buffer,
    bind_groups: Vec<wgpu::BindGroup>,
    shadow_uniform_buffer: wgpu::Buffer,
    shadow_bind_group: wgpu::BindGroup,
    /// Cache key to skip re-rasterisation when the tile hasn't changed.
    tile_id: (Suit, u8, Option<crate::core::tile::TileEnhancement>),
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
    tile_quad_pipeline: wgpu::RenderPipeline,
    light_beam_pipeline: wgpu::RenderPipeline,
    flame_pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    vertex_buffer: wgpu::Buffer,
    #[allow(dead_code)]
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
    /// Smoothed hover envelope for the discard bowl, in [0, 1]. Eased
    /// toward the per-frame target hover (0 or 1) so the lift + tilt
    /// animation runs in both directions instead of snapping.
    bowl_hover_anim: f32,
    /// Smoothed hover envelope for the bronze mirror, in [0, 1]. Same
    /// easing convention as `bowl_hover_anim`.
    mirror_hover_anim: f32,
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
    #[allow(dead_code)]
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
    /// Pipeline for procedural scene props (candles, table). Shares the
    /// `point_lights_layout` (group 1) with the tile pipeline.
    lit_mesh_pipeline: wgpu::RenderPipeline,
    /// 1×1 white texture used as a placeholder albedo for procedural meshes
    /// that don't sample from a texture.
    #[allow(dead_code)]
    lit_mesh_white_tex: wgpu::Texture,
    #[allow(dead_code)]
    lit_mesh_white_view: wgpu::TextureView,
    /// Linear-format heightmap texture for the shop coin faces. Bound at
    /// slot 1 of every coin instance — sampled by the metal branch in
    /// `lit_mesh.wgsl` to perturb the surface normal so the engraved
    /// Chinese cash-coin face catches the candle highlights. Kept on the
    /// renderer purely so the GPU resource outlives the bind groups that
    /// reference it.
    #[allow(dead_code)]
    lit_mesh_coin_height_tex: wgpu::Texture,
    #[allow(dead_code)]
    lit_mesh_coin_height_view: wgpu::TextureView,
    /// Per-kind procedural heightmap textures for talisman tablets. Indexed
    /// by `TalismanKind::all()` order (Jade=0, Pearl=1, Gilded=2,
    /// Polychrome=3). The talisman shader branch samples these as a
    /// grayscale heightfield to perturb the surface normal.
    #[allow(dead_code)]
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
    relic_box_mesh: LitMeshGpu,
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
    /// Pre-allocated per-pack instances (same mesh + pipeline as relics).
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
    peg_block_mesh: LitMeshGpu,
    dora_stand_mesh: LitMeshGpu,
    plaque_instances: Vec<LitMeshInstance>,
    ofuda_instances: Vec<LitMeshInstance>,
    yaku_tablet_instances: Vec<LitMeshInstance>,
    wood_tablet_instances: Vec<LitMeshInstance>,
    bowl_instances: Vec<LitMeshInstance>,
    mirror_instances: Vec<LitMeshInstance>,
    peg_block_instances: Vec<LitMeshInstance>,
    /// Per-peg cylinder instances. The peg cylinders reuse `coin_mesh`
    /// (geometry) but get their own slot pool so they don't compete with the
    /// shop scene's coin pile for slots.
    peg_instances: Vec<LitMeshInstance>,
    /// Per-wall-tile instances for the back-of-table facedown stack. Reuses
    /// `bone_tablet_mesh` for phase 1 (a plain box) — phase 7 may swap to the
    /// real tile mesh.
    wall_tile_instances: Vec<LitMeshInstance>,
    dora_stand_instances: Vec<LitMeshInstance>,
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
    last_debug_pickables: Vec<(&'static str, Mat4, glam::Vec3, f32)>,

    // ── Shadow mapping ─────────────────────────────────────────────────
    /// Fixed-size depth texture written by the shadow pre-pass and sampled
    /// by every 3D shader through `shadow_sample_bind_group`.
    #[allow(dead_code)]
    shadow_map_texture: wgpu::Texture,
    #[allow(dead_code)]
    shadow_map_view: wgpu::TextureView,
    /// Bind-group layout for per-caster uniforms (group 0 of the shadow
    /// pipeline). Each `LitMeshInstance` and `HandTileGpu` owns one bind
    /// group built against this layout.
    shadow_caster_layout: wgpu::BindGroupLayout,
    /// Bind-group layout for the frame-shared shadow sampling group
    /// (group 2 of every 3D scene pipeline).
    #[allow(dead_code)]
    shadow_sample_layout: wgpu::BindGroupLayout,
    /// Frame-shared uniform: light_view_proj + (enabled, bias, texel size).
    #[allow(dead_code)]
    shadow_globals_buffer: wgpu::Buffer,
    /// Frame-shared bind group bound as group 2 on every 3D draw in the
    /// main pass. Wraps the depth texture, comparison sampler, and
    /// `shadow_globals_buffer`.
    shadow_sample_bind_group: wgpu::BindGroup,
    /// Depth-only pipeline used for the shadow pre-pass. Both lit-mesh
    /// casters and hand tiles share this pipeline because both vertex
    /// layouts start with `position : vec3<f32>` at offset 0.
    #[allow(dead_code)]
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
#[derive(Clone, Copy, Debug)]
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
/// Maximum number of explicit auxiliary dishes per frame (the shop uses 2:
/// the relic dish and the coin dish).
/// Maximum number of shrine instances per frame (pick-blind uses 3: Small,
/// Big, Boss). Truncated silently.
pub const MAX_SHRINE_SLOTS: usize = 4;
/// Maximum number of hanging plaques per frame (gameplay uses 1).
pub const MAX_PLAQUE_SLOTS: usize = 2;
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
/// Maximum number of peg blocks per frame (gameplay uses 1).
pub const MAX_PEG_BLOCK_SLOTS: usize = 2;
/// Maximum number of individual peg cylinders rendered per frame across all
/// peg blocks. Each block has plays_max + discards_max pegs; this caps the
/// visible total.
pub const MAX_PEG_SLOTS: usize = 32;
/// Maximum number of facedown wall tiles drawn at the back of the table.
pub const MAX_WALL_TILE_SLOTS: usize = 80;
/// Maximum number of dora stands per frame.
pub const MAX_DORA_STAND_SLOTS: usize = 2;
/// Maximum number of cascade scoring tokens per frame (chips + mult).
pub const MAX_CASCADE_TOKEN_SLOTS: usize = 4;
/// Maximum number of physical falling-bone instances in flight at once.
/// Sized to comfortably hold a multi-step cascade's worth of bursts (each
/// scoring step spawns a small handful) without overflowing the pool.
pub const MAX_FALLING_BONE_SLOTS: usize = 192;
/// Maximum number of in-flight 3D extruded-glyph score popups. A single
/// cascade rarely fires more than 8-10 steps, so 32 is plenty for the
/// per-step popups plus the running-total readout that holds across the
/// final beat.
pub const MAX_EXTRUDED_GLYPH_SLOTS: usize = 32;

/// Pre-loaded relic icon texture + bind group for the image pipeline.
struct RelicTextureGpu {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    /// Bind group for the 2D image pipeline (collection screen).
    bind_group: wgpu::BindGroup,
    /// Texture view for binding into lit-mesh material bind groups (3D boxes).
    view: wgpu::TextureView,
}

/// Decoded relic image data sent from the background loader thread.
struct DecodedRelicImage {
    id: RelicId,
    name: &'static str,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

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

/// Spawn a background thread that decodes all relic PNGs and sends the RGBA
/// data back over a channel.  The main thread uploads to the GPU as results
/// arrive (see `poll_relic_textures`).
fn spawn_relic_loader() -> mpsc::Receiver<DecodedRelicImage> {
    use crate::core::relic::all_relic_defs;

    let (tx, rx) = mpsc::channel();

    // Collect the static data we need before moving into the thread.
    let defs: Vec<(RelicId, &'static str, String)> = all_relic_defs()
        .iter()
        .map(|d| {
            let asset_path = format!("textures/relics/{}", d.id.asset_filename());
            (d.id, d.name, asset_path)
        })
        .collect();

    std::thread::Builder::new()
        .name("relic-loader".into())
        .spawn(move || {
            let t_thread = Instant::now();
            let mut decoded = 0usize;
            let mut decode_time = std::time::Duration::ZERO;
            for (id, name, asset_path) in defs {
                let bytes = match crate::asset_path::get(&asset_path) {
                    Some(file) => file.data.to_vec(),
                    None => {
                        log::warn!("relic icon not found in embedded assets: {asset_path}");
                        continue;
                    }
                };
                let t_decode = Instant::now();
                let img = match image::load_from_memory(&bytes) {
                    Ok(img) => img.into_rgba8(),
                    Err(e) => {
                        log::warn!("failed to decode relic icon {asset_path}: {e}");
                        continue;
                    }
                };
                decode_time += t_decode.elapsed();
                decoded += 1;
                let (w, h) = img.dimensions();
                let msg = DecodedRelicImage {
                    id,
                    name,
                    rgba: img.into_raw(),
                    width: w,
                    height: h,
                };
                if tx.send(msg).is_err() {
                    break; // receiver dropped, renderer shut down
                }
            }
            log::info!(
                "relic-loader thread finished: decoded {decoded} images in {decode_time:?} (thread total {:?})",
                t_thread.elapsed(),
            );
        })
        .expect("failed to spawn relic-loader thread");

    rx
}

/// Load tile-pack box art textures synchronously at init. There are at most 7
/// packs and only a handful have art, so the blocking decode is trivial.
fn load_pack_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    text_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> HashMap<TilePackKind, RelicTextureGpu> {
    let mut map = HashMap::new();
    for &kind in TilePackKind::all() {
        let asset_path = format!("textures/packs/{}", kind.asset_filename());
        let bytes = match crate::asset_path::get(&asset_path) {
            Some(file) => file.data.to_vec(),
            None => {
                log::debug!("pack texture not found (optional): {asset_path}");
                continue;
            }
        };
        let img = match image::load_from_memory(&bytes) {
            Ok(img) => img.into_rgba8(),
            Err(e) => {
                log::warn!("failed to decode pack texture {asset_path}: {e}");
                continue;
            }
        };
        let (w, h) = img.dimensions();
        let (tex, view) = upload_rgba_texture(device, queue, kind.name(), img.as_raw(), w, h);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(kind.name()),
            layout: text_layout,
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
        map.insert(
            kind,
            RelicTextureGpu {
                view,
                texture: tex,
                bind_group,
            },
        );
    }
    log::info!("loaded {} pack textures synchronously", map.len());
    map
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

// ---------------------------------------------------------------------------
// Per-tile GPU resource builder (free function avoids double-borrow of `self`)
// ---------------------------------------------------------------------------

fn make_hand_tile_gpu(
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
) -> HandTileGpu {
    let identity = Mat4::IDENTITY;
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("hand-tile-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    // The tile face is 0.734 wide × 1.0 tall in local coords (see tile_3d.wgsl
    // — local Z is the short on-screen-horizontal axis, local X is the long
    // on-screen-vertical axis). Match that aspect in the texture so the GPU
    // stretching doesn't distort the rasterised glyphs.
    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    let rgba = rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set);
    let (decal_texture, decal_view) =
        upload_rgba_texture(device, queue, "hand-tile-decal", &rgba, DECAL_W, DECAL_H);

    let bind_groups: Vec<wgpu::BindGroup> = primitives
        .iter()
        .map(|prim| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hand-tile-bg"),
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

    // Outline shell uniform + matching bind groups. The outline pipeline
    // only samples binding 0 (camera uniform) but we have to provide the
    // texture/sampler bindings to satisfy the shared layout.
    let outline_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("hand-tile-outline-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let outline_bind_groups: Vec<wgpu::BindGroup> = primitives
        .iter()
        .map(|prim| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hand-tile-outline-bg"),
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

    // Per-tile shadow caster uniform — written each frame the tile is
    // visible with the same model matrix as the main uniform.
    let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("hand-tile-shadow-uniform"),
        contents: bytemuck::bytes_of(&ShadowCasterUniform {
            light_view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hand-tile-shadow-bg"),
        layout: shadow_caster_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: shadow_uniform_buffer.as_entire_binding(),
        }],
    });

    let symbol = tile_short_label(tile);
    let suit_emoji = tile_suit_emoji(tile).to_string();
    let suit_color = tile.suit_color();
    HandTileGpu {
        uniform_buffer,
        bind_groups,
        outline_uniform_buffer,
        outline_bind_groups,
        shadow_uniform_buffer,
        shadow_bind_group,
        tile_id: (tile.suit, tile.rank, tile.enhancement),
        symbol,
        suit_emoji,
        suit_color,
        decal_texture,
    }
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
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    let rgba = rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set);
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
        shadow_uniform_buffer,
        shadow_bind_group,
        tile_id: (tile.suit, tile.rank, tile.enhancement),
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
        let mut required_features = wgpu::Features::empty();
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

        // Tile quad pipeline — SDF rounded rect with ivory/bamboo look.
        let tile_quad_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile_quad.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/tile_quad.wgsl"
                ))
                .into(),
            ),
        });

        let tile_quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile-quad-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &tile_quad_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tile_quad_shader,
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

        // Light beam pipeline — volumetric directional light with procedural dust.
        let light_beam_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("light_beam.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/light_beam.wgsl"
                ))
                .into(),
            ),
        });

        let light_beam_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("light-beam-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &light_beam_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &light_beam_shader,
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
                    // leave a thin gold rim. The tile model matrix has
                    // determinant −1 (the local→world basis is a
                    // reflection that swaps local X↔Z), which flips
                    // post-transform winding — so wgpu's `Back` face here
                    // corresponds to the geometric outward face of the
                    // shell. Culling `Back` therefore leaves the inward
                    // (away-from-camera) shell faces drawn, which is what
                    // we want.
                    cull_mode: Some(wgpu::Face::Back),
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
        let pack_textures_map =
            load_pack_textures(&device, &queue, &text_bind_group_layout, &tile_sampler);
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
        let relic_box_mesh = LitMeshGpu::new(&device, &build_unit_box_mesh(), "relic-box");
        let ribbon_mesh = LitMeshGpu::new(&device, &build_ribbon_mesh(), "ribbon");
        let coin_mesh = LitMeshGpu::new(&device, &build_coin_mesh(), "coin");
        let talisman_mesh = LitMeshGpu::new(&device, &build_talisman_mesh(), "talisman");
        let cabinet_mesh = LitMeshGpu::new(&device, &build_curio_cabinet_mesh(), "curio-cabinet");
        let shrine_mesh = LitMeshGpu::new(&device, &build_shrine_mesh(), "shrine");
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
        let peg_block_mesh = LitMeshGpu::new(&device, &build_peg_block_mesh(), "peg-block");
        let dora_stand_mesh = LitMeshGpu::new(&device, &build_dora_stand_mesh(), "dora-stand");
        // Shared 1×1 white texture for procedural meshes that don't sample.
        let (lit_mesh_white_tex, lit_mesh_white_view) = white_albedo(&device, &queue);

        // Pre-allocate candle slots (matches the gameplay layout's ambient
        // candles plus a "footlight" candle in front of the camera that
        // illuminates the bottom row of yaku/wood tablets — without it the
        // 3D action row falls completely outside every other candle's pool
        // and reads as a black silhouette). Each slot owns two instances:
        // wax + wick.
        const NUM_CANDLE_SLOTS: usize = 5;
        let mut candle_instances: Vec<[LitMeshInstance; 2]> = Vec::with_capacity(NUM_CANDLE_SLOTS);
        for _ in 0..NUM_CANDLE_SLOTS {
            candle_instances.push([
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &shadow_caster_layout,
                    &lit_mesh_white_view,
                    &tile_sampler,
                ),
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &shadow_caster_layout,
                    &lit_mesh_white_view,
                    &tile_sampler,
                ),
            ]);
        }
        let table_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &tile_sampler,
        );
        let dish_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &tile_sampler,
        );
        let mut relic_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_RELIC_SLOTS);
        for _ in 0..MAX_RELIC_SLOTS {
            relic_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
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
                &tile_sampler,
            ));
        }
        // Per-kind heightmap textures for talisman tablets. Each is a PNG
        // asset loaded from assets/textures/ and uploaded as a linear RGBA8
        // texture. Falls back to a flat mid-gray 1×1 if the asset is missing.
        let talisman_height_paths = [
            ("textures/talisman_jade.png", "talisman-jade-hm"),
            ("textures/talisman_pearl.png", "talisman-pearl-hm"),
            ("textures/talisman_gilded.png", "talisman-gilded-hm"),
            ("textures/talisman_polychrome.png", "talisman-polychrome-hm"),
            ("textures/talisman_kiln.png", "talisman-kiln-hm"),
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
                &tile_sampler,
            ));
        }
        let cabinet_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &tile_sampler,
        );
        let mut shrine_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_SHRINE_SLOTS);
        for _ in 0..MAX_SHRINE_SLOTS {
            shrine_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
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
                        &tile_sampler,
                    )
                })
                .collect()
        };
        let plaque_instances = make_pool(MAX_PLAQUE_SLOTS);
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
                    &tile_sampler,
                )
            })
            .collect();
        let peg_block_instances = make_pool(MAX_PEG_BLOCK_SLOTS);
        let peg_instances = make_pool(MAX_PEG_SLOTS);
        let wall_tile_instances = make_pool(MAX_WALL_TILE_SLOTS);
        let dora_stand_instances = make_pool(MAX_DORA_STAND_SLOTS);
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
            tile_quad_pipeline,
            light_beam_pipeline,
            flame_pipeline,
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
            peg_block_mesh,
            dora_stand_mesh,
            plaque_instances,
            ofuda_instances,
            yaku_tablet_instances,
            wood_tablet_instances,
            bowl_instances,
            mirror_instances,
            peg_block_instances,
            peg_instances,
            wall_tile_instances,
            dora_stand_instances,
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
            last_frame: Instant::now(),
            frame_dt: 0.0,
            bowl_hover_anim: 0.0,
            mirror_hover_anim: 0.0,
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
            lit_mesh_pipeline,
            lit_mesh_white_tex,
            lit_mesh_white_view,
            lit_mesh_coin_height_tex,
            lit_mesh_coin_height_view,
            talisman_height_textures,
            talisman_height_views,
            talisman_slot_kind,
            candle_wax_mesh,
            candle_wick_mesh,
            table_mesh,
            dish_mesh,
            relic_box_mesh,
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

    #[allow(dead_code)]
    pub fn has_tile_mesh(&self) -> bool {
        !self.tile_primitives.is_empty()
    }

    /// Returns `true` while background asset loading (relic/background textures)
    /// is still in progress.
    pub fn is_loading(&self) -> bool {
        self.relic_rx.is_some() || self.background_rx.is_some()
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
                    let (tex, view) = upload_rgba_texture(
                        &self.device,
                        &self.queue,
                        img.name,
                        &img.rgba,
                        img.width,
                        img.height,
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
    /// LOCAL_*_EXTENT / 2). Because the tile mesh is approximately cuboid,
    /// this is effectively a raycast against the tile silhouette.
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
                consider(ShopHit::Dish(*pid), 0.5);
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
    pub fn pick_debug_object(&self, cursor_x: f32, cursor_y: f32) -> Option<&'static str> {
        // Hand tiles first — they have their own dedicated picker that
        // already handles per-tile OBBs.
        if let Some(idx) = self.pick_hand_tile(cursor_x, cursor_y) {
            // Leak the slot index into a static-ish label. We allocate a
            // tiny pool of pre-formatted strings; for indices outside the
            // pool just fall back to a generic name.
            const HAND_TILE_NAMES: [&str; 14] = [
                "HandTile[0]",
                "HandTile[1]",
                "HandTile[2]",
                "HandTile[3]",
                "HandTile[4]",
                "HandTile[5]",
                "HandTile[6]",
                "HandTile[7]",
                "HandTile[8]",
                "HandTile[9]",
                "HandTile[10]",
                "HandTile[11]",
                "HandTile[12]",
                "HandTile[13]",
            ];
            return Some(HAND_TILE_NAMES.get(idx).copied().unwrap_or("HandTile"));
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

        let mut best: Option<(&'static str, f32)> = None;
        for &(name, model, half, oy) in &self.last_debug_pickables {
            if let Some(t) = slab_test(model, half, oy) {
                match best {
                    Some((_, bt)) if t >= bt => {}
                    _ => best = Some((name, t)),
                }
            }
        }
        best.map(|(n, _)| n)
    }

    /// Ensure `hand_tiles` matches `tiles`.
    ///
    /// Only re-rasterises decals for slots whose tile identity (suit + rank)
    /// has changed, so unchanged tiles keep their GPU textures.
    pub fn update_hand_tiles(&mut self, tiles: &[Tile]) {
        // Build old uid → slot index map before we modify anything.
        let old_uid_to_slot: std::collections::HashMap<u32, usize> = self
            .tile_uids
            .iter()
            .enumerate()
            .filter(|&(_, &uid)| uid != u32::MAX)
            .map(|(i, &uid)| (uid, i))
            .collect();

        self.hand_tiles.truncate(tiles.len());
        self.focus_t.resize(tiles.len(), 0.0);
        self.tile_anim_y.resize(tiles.len(), 0.0);
        self.tile_anim_x.resize(tiles.len(), 0.0);
        self.tile_uids.resize(tiles.len(), u32::MAX);

        // Count truly new tiles (not previously in hand) for staggered draw animation.
        let mut new_tile_order: usize = 0;

        for (i, tile) in tiles.iter().enumerate() {
            let id = (tile.suit, tile.rank, tile.enhancement);
            let uid = tile.id;
            let is_new = self.tile_uids[i] != uid;
            self.tile_uids[i] = uid;

            // Re-rasterise if either the tile identity (uid) changed OR the
            // cached visual key changed (e.g. a talisman was stamped onto an
            // existing tile, leaving uid the same but enhancement different).
            if !is_new
                && self
                    .hand_tiles
                    .get(i)
                    .map(|d| d.tile_id == id)
                    .unwrap_or(false)
            {
                continue;
            }

            if is_new {
                if let Some(&old_slot) = old_uid_to_slot.get(&uid) {
                    // Tile existed before but moved slots (sort). Animate horizontally.
                    let slot_offset = (old_slot as f32) - (i as f32);
                    self.tile_anim_x[i] = slot_offset;
                    // Don't set Y animation — it's not a new tile, just repositioned.
                } else {
                    // Truly new tile (drawn from wall). Stagger the Y offset.
                    let stagger = new_tile_order as f32 * 30.0;
                    self.tile_anim_y[i] = 120.0 + stagger;
                    new_tile_order += 1;
                }
            }
            let htg = make_hand_tile_gpu(
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
            if i < self.hand_tiles.len() {
                self.hand_tiles[i] = htg;
            } else {
                self.hand_tiles.push(htg);
            }
        }
    }

    /// Spawn discard departure animations for the given tile slot indices.
    /// Call this *before* `update_hand_tiles` removes the tiles so we can
    /// capture their visual data. Uses `prev_hand_slots` for screen
    /// positions and `last_projected_bowl_rect` (one-frame stale) for the
    /// discard river's screen-space target.
    ///
    /// Trajectory is two-phase: a quadratic Bezier arc from the hand slot
    /// up over an apex and down into the river, followed by a downstream
    /// drift along +X that fades the tile out. `depart_lifetime` controls
    /// the *total* combined duration so the existing
    /// `cascade_tuning.depart_lifetime_ms` debug knob still scales the
    /// whole animation.
    pub fn depart_tiles(
        &mut self,
        indices: &[usize],
        depart_lifetime: f32,
        tile_preset: crate::persistence::TilePreset,
    ) {
        let mut rng_seed = self.creation_time.elapsed().as_nanos() as u32;
        let cheap_rand = |seed: &mut u32| -> f32 {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            (*seed as f32) / u32::MAX as f32
        };

        // River target = projected bowl rect from the previous frame. If
        // the renderer hasn't drawn one yet (e.g. very first frame after a
        // scene transition) fall back to a point off-screen above the
        // hand so the tile still arcs out of view instead of stalling.
        let river_rect = self.proj.bowl_rect;
        let (river_cx, river_cy) = river_rect
            .map(|r| (r[0] + r[2] * 0.5, r[1] + r[3] * 0.5))
            .unwrap_or((self.size.width as f32 * 0.5, -100.0));

        // Roughly 40% of the total lifetime is the arc into the river;
        // the rest is the drift+fade. Tunable later if the splash beat
        // needs to land earlier.
        let arc_dur = (depart_lifetime * 0.45).max(0.18);
        let drift_dur = (depart_lifetime - arc_dur).max(0.20);

        for (order, &idx) in indices.iter().enumerate() {
            let Some(htg) = self.hand_tiles.get(idx) else {
                continue;
            };
            let Some(&(sx, sy, sw, sh)) = self.prev_hand_slots.get(idx) else {
                continue;
            };

            // Compute the tile rect (matching render logic). Uses the
            // current tile preset's face aspect so departing tiles share
            // shape with the in-hand tiles they came from.
            let tile_w = sw * 0.85;
            let tile_h = tile_w * tile_preset.face_long_ratio();
            let tx = sx + (sw - tile_w) * 0.5;
            let ty = sy + (sh - tile_h) * 0.5;

            // Per-tile splash jitter — keep multiple discards from
            // landing on the exact same pixel.
            let jitter_x = (cheap_rand(&mut rng_seed) - 0.5) * sw * 0.4;
            let jitter_y = (cheap_rand(&mut rng_seed) - 0.5) * sh * 0.25;

            self.departing_tiles.push(DepartingTile {
                symbol: htg.symbol.clone(),
                suit_emoji: htg.suit_emoji.clone(),
                suit_color: htg.suit_color,
                start_rect: [tx, ty, tile_w, tile_h],
                river_target: (river_cx + jitter_x, river_cy + jitter_y),
                // River flows in +X (left → right) on screen. Drift speed
                // is comfortable but not zippy — fast enough that the
                // tile clearly leaves the splash point in the ~0.4s
                // before it fades out.
                drift_dir: (1.0, 0.0),
                drift_speed: 140.0 + cheap_rand(&mut rng_seed) * 60.0,
                arc_dur,
                drift_dur,
                elapsed: -(order as f32) * 0.06, // stagger departures slightly
                lifetime: arc_dur + drift_dur,
            });
        }
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

        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [new_size.width as f32, new_size.height as f32],
                time: self.creation_time.elapsed().as_secs_f32(),
                // Gamma will be re-uploaded on the next render() call.
                gamma: 1.0,
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

        let hand_slots: &[(f32, f32, f32, f32)] = &frame.hand_slots;
        let focus = frame.focus;
        let selected: &[bool] = &frame.selected_tiles;
        let hint_indices: &[usize] = &frame.hint_indices;
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
        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [self.size.width as f32, self.size.height as f32],
                time: self.creation_time.elapsed().as_secs_f32(),
                gamma: gamma.max(0.01),
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
        // The 3D world is a horizontal table in the XZ plane (y=0). The
        // player sits in front of the table (large +Z), eyes above the
        // table, looking down and slightly forward. We map the layout's
        // pixel coordinates onto the table's surface so the existing
        // pixel-space layout still drives where things go:
        //
        //   world_x =  pixel_x - w * 0.5
        //   world_z =  pixel_y - h * 0.5     (pixel y grows downward, so
        //                                     bottom of screen → +z, near
        //                                     player; top of screen → -z,
        //                                     far edge of the table)
        //   world_y =  height above the table (0 = sitting on the wood)
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
            let eye_height = h * 0.55;
            let eye_back = h * 1.0;
            (
                glam::Vec3::new(0.0, eye_height, eye_back),
                glam::Vec3::new(0.0, h * 0.05, -h * 0.10),
                55.0_f32.to_radians(),
            )
        };
        let up_v = frame
            .camera_override
            .as_ref()
            .map(|c| glam::Vec3::from_array(c.up))
            .unwrap_or(glam::Vec3::Y);
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

        // Helper: map a layout pixel position onto the table-plane world.
        let pixel_to_world = |px: f32, py: f32, world_y: f32| -> glam::Vec3 {
            glam::Vec3::new(px - w * 0.5, world_y, py - h * 0.5)
        };
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
                let model = Mat4::from_translation(center) * Mat4::from_scale(*scale);
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
                        let base_world = pixel_to_world(p.world_pos[0], p.world_pos[1], 0.0);
                        let tip_world = pixel_to_world(
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
                            let g_world = pixel_to_world(g.center_px.0, g.center_px.1, g.lift);
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

        // Tile-mesh local extents (after `normalize_mesh` in tile_glb.rs):
        //   local X — long face axis  (extent ~1.000) → table-Z (front-back)
        //   local Z — short face axis (extent ~0.734) → table-X (left-right)
        //   local Y — thickness        (extent ~0.424) → world Y (up off table)
        //
        // The new basis maps the mesh into a tile lying flat with its
        // front face (+Y normal) pointing straight up at the camera.
        let tile_basis = Mat4::from_cols(
            glam::Vec4::new(0.0, 0.0, 1.0, 0.0), // local X → world +Z (front-back)
            glam::Vec4::new(0.0, 1.0, 0.0, 0.0), // local Y → world +Y (face up)
            glam::Vec4::new(1.0, 0.0, 0.0, 0.0), // local Z → world +X (left-right)
            glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
        );

        {
            for (i, htg) in self.hand_tiles.iter().enumerate() {
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

                // Tile center in pixel-layout coords.
                let cx_px = sx + sw * 0.5 + slide_x_slots * sw;
                // The slide_y residual still pushes the tile back briefly
                // for new-tile entry; in table-space that becomes a +z push
                // (further from the player) which reads as the tile sliding
                // in across the wood toward its final spot.
                let cy_px = sy + sh * 0.5 + slide_y;

                // World position: laid flat just above the table.
                let world_y_lift = tile_thickness_px * 0.5 + 4.0;
                let world = pixel_to_world(cx_px, cy_px, world_y_lift);

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
                                );
                            }
                        }
                    }
                    self.prev_tile_world.insert(uid, world);
                }

                // Tilt rotation, computed once and reused for both the
                // model matrix below and the overlay-anchor projection.
                // Pivot is at the tile's bottom-front corner in world
                // space (after basis * scale): bottom in world Y,
                // front (toward camera) in world +Z.
                let tilt_angle = 22.0_f32.to_radians();
                let tilt_pivot = glam::Vec3::new(0.0, -tile_thickness_px * 0.5, tile_long_px * 0.5);
                let tilt = Mat4::from_translation(tilt_pivot)
                    * Mat4::from_rotation_x(tilt_angle)
                    * Mat4::from_translation(-tilt_pivot);

                // Helper: take a point expressed *relative to the tile
                // center* (in world axes — z is front-back, y is up),
                // tilt it, translate to the world tile position, and
                // project to screen.
                let tilted_to_screen = |local: glam::Vec3| -> (f32, f32) {
                    let tilted = tilt.transform_point3(local);
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
                let hx = tile_short_px * 0.5;
                let hy = tile_thickness_px * 0.5;
                let hz = tile_long_px * 0.5;
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
                let mut min_x = f32::INFINITY;
                let mut min_y = f32::INFINITY;
                let mut max_x = f32::NEG_INFINITY;
                let mut max_y = f32::NEG_INFINITY;
                for c in corners {
                    let (px, py) = tilted_to_screen(c);
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
                        tile_long_px / LOCAL_X_EXTENT, // local X (long) → world Z (front-back)
                        tile_thickness_px / LOCAL_Y_EXTENT, // local Y → world Y (thickness)
                        tile_short_px / LOCAL_Z_EXTENT, // local Z (short) → world X (left-right)
                    );
                    // Pack enhancement kind into .z so the tile shader can
                    // apply fresnel-masked sheen effects per-enhancement.
                    let mut bcf = self.tile_base_color_factor;
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
                        let outline_model = Mat4::from_translation(world)
                            * tilt
                            * tile_basis
                            * Mat4::from_scale(outline_scale);
                        self.queue.write_buffer(
                            &htg.outline_uniform_buffer,
                            0,
                            bytemuck::bytes_of(&CameraUniform {
                                view_proj: view_proj_arr,
                                model: outline_model.to_cols_array(),
                                base_color_factor: bcf,
                            }),
                        );
                    }
                    // `tilt` was computed above the projection block so
                    // both the model matrix and the overlay anchors share
                    // the same rotation.
                    let model =
                        Mat4::from_translation(world) * tilt * tile_basis * Mat4::from_scale(scale);
                    // Snapshot for next frame's cursor pick.
                    tile_pick_models.push((i, model));
                    self.queue.write_buffer(
                        &htg.uniform_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_proj: view_proj_arr,
                            model: model.to_cols_array(),
                            base_color_factor: bcf,
                        }),
                    );
                }
            }
        }

        // Snapshot the projected tile rects for the next frame's scene draw
        // (used by hover tooltips and any other 2D HUD that needs to anchor
        // to the actual visible tile).
        self.proj.hand_rects = tile_3d_rects.clone();
        self.last_pick_models = tile_pick_models.clone();
        self.last_pick_camera = Some(PickCamera {
            inv_view_proj: view_proj.inverse(),
            viewport_w: w,
            viewport_h: h,
        });

        // Tile hints are now real green PointLights pushed by the gameplay
        // scene into `frame.point_lights` (see the hint-lights block in
        // `scenes/gameplay.rs`). The 2D fake "light beam" overlay that
        // used to live here was removed in favour of letting the real
        // lighting model do the work — the hinted tile picks up a green
        // top-down pool through the same shader path as the candles.
        let _ = hint_indices;
        let light_beams: Vec<GpuInstance> = Vec::new();

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

        // Tile quad instance buffer (separate from scene instances — uses tile_quad_pipeline).
        let tile_instance_buffer = if tile_quads.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile-instances"),
                        contents: bytemuck::cast_slice(&tile_quads),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        // Light beam instance buffer (rendered behind tiles).
        let light_beam_buffer = if light_beams.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("light-beams"),
                        contents: bytemuck::cast_slice(&light_beams),
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
            let tw = (lbl.rect[2] as u32).max(1);
            let th = (lbl.rect[3] as u32).max(1);
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
        struct RelicDraw {
            inst_buf: wgpu::Buffer,
            relic_id: RelicId,
        }

        enum RenderOp {
            Background(BackgroundId),
            Table,
            Dish,
            RelicBatch(usize),    // index into `relic_batches`
            PackBatch(usize),     // index into `pack_batches`
            CandleBatch(usize),   // index into `candle_batches`
            DishExplicit(usize),  // index into `aux_dish_cmds`
            CurioCabinet, // single instance — only the most-recent CurioCabinet cmd is drawn
            ShrineBatch(usize), // index into `shrine_batches`
            ZodiacBatch(usize), // index into `ribbon_batches`
            TalismanBatch(usize), // index into `talisman_batches`
            CoinBatch(usize), // index into `coin_batches`
            QuadBatch { buf_idx: usize, count: u32 },
            FlameBatch { buf_idx: usize, count: u32 },
            TextDraw(usize),
            RelicIconDraw(usize),
            HandTileBackdrop,
            HandTileFaces,
            FluidSmoke,
            // Skeuomorphic gameplay HUD (phase 1).
            Plaque(usize),             // index into `plaque_cmds`
            Ofuda(usize),              // index into `ofuda_cmds`
            YakuTabletBatch(usize),    // index into `yaku_tablet_batches`
            WoodTabletBatch(usize),    // index into `wood_tablet_batches`
            Bowl(usize),               // index into `bowl_cmds`
            Mirror(usize),             // index into `mirror_cmds`
            PegBlock(usize),           // index into `peg_block_cmds`
            WallStack(usize),          // index into `wall_stack_cmds`
            DoraStand(usize),          // index into `dora_stand_cmds`
            CascadeTokenBatch(usize),  // index into `cascade_token_batches`
            FallingBoneBatch(usize),   // index into `falling_bone_batches`
            ExtrudedGlyphBatch(usize), // index into `extruded_glyph_batches`
            ShowcaseTileBatch(usize),  // index into `showcase_tile_batches`
        }

        let mut quad_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut flame_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut text_draws: Vec<TextDraw> = Vec::new();
        let mut relic_draws: Vec<RelicDraw> = Vec::new();
        let mut candle_batches: Vec<&[CandlePlacement]> = Vec::new();
        let mut relic_batches: Vec<&[RelicPlacement]> = Vec::new();
        let mut pack_batches: Vec<&[PackPlacement]> = Vec::new();
        let mut ribbon_batches: Vec<&[ZodiacRibbonPlacement]> = Vec::new();
        let mut talisman_batches: Vec<&[TalismanPlacement]> = Vec::new();
        let mut coin_batches: Vec<&[CoinPlacement]> = Vec::new();
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
        let mut peg_block_cmds: Vec<&PegBlockPlacement> = Vec::new();
        let mut wall_stack_cmds: Vec<&WallStackPlacement> = Vec::new();
        let mut dora_stand_cmds: Vec<&DoraStandPlacement> = Vec::new();
        let mut cascade_token_batches: Vec<&[CascadeTokenPlacement]> = Vec::new();
        let mut falling_bone_batches: Vec<&[FallingBonePlacement]> = Vec::new();
        let mut extruded_glyph_batches: Vec<&[ExtrudedGlyphPlacement]> = Vec::new();
        let mut showcase_tile_batches: Vec<&[ShowcaseTilePlacement]> = Vec::new();
        let mut ops: Vec<RenderOp> = Vec::new();

        let mut i = 0;
        while i < frame.cmds.len() {
            match &frame.cmds[i] {
                DrawCmd::Background(id) => {
                    ops.push(RenderOp::Background(*id));
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
                DrawCmd::PegBlock(p) => {
                    let idx = peg_block_cmds.len();
                    peg_block_cmds.push(p);
                    ops.push(RenderOp::PegBlock(idx));
                    i += 1;
                }
                DrawCmd::WallStack(p) => {
                    let idx = wall_stack_cmds.len();
                    wall_stack_cmds.push(p);
                    ops.push(RenderOp::WallStack(idx));
                    i += 1;
                }
                DrawCmd::DoraStand(p) => {
                    let idx = dora_stand_cmds.len();
                    dora_stand_cmds.push(p);
                    ops.push(RenderOp::DoraStand(idx));
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
                DrawCmd::HandTileBackdrop => {
                    ops.push(RenderOp::HandTileBackdrop);
                    i += 1;
                }
                DrawCmd::HandTileFaces => {
                    ops.push(RenderOp::HandTileFaces);
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
                        let idx = relic_draws.len();
                        relic_draws.push(RelicDraw {
                            inst_buf,
                            relic_id: icon.relic_id,
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
            // Horizontal table: the mesh is built in XY (normal +Z), so we
            // rotate -90° around X to lay it flat with normal +Y. Scaled
            // wildly larger than the camera's far plane so the wood plane
            // extends to the visible horizon — the lit_mesh shader now
            // tiles the procedural grain in world XZ, so making the table
            // huge does not stretch the rings the way it used to.
            let table_extent = h * 30.0;
            let table_w = table_extent;
            let table_d = table_extent;
            let model = Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 0.0))
                * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
                * Mat4::from_scale(glam::Vec3::new(table_w, table_d, 1.0));
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

        // Candles: scenes pass `world_pos = (pixel_x, pixel_y, world_y_lift)`
        // — we map pixel x/y onto the table plane and use world_y as the
        // base height above the wood (usually 0 so the candle sits on it).
        for batch in &candle_batches {
            for (slot_i, placement) in batch.iter().enumerate() {
                let Some(instances) = self.candle_instances.get(slot_i) else {
                    break;
                };
                let base = pixel_to_world(
                    placement.world_pos[0],
                    placement.world_pos[1],
                    placement.world_pos[2],
                );
                let s = placement.scale;
                let model = Mat4::from_translation(base)
                    * Mat4::from_scale(glam::Vec3::new(s, s * placement.height_scale, s));
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
                    "Candle",
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
        self.proj.relic_rects.clear();
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
                    p.world_pos[0],
                    p.world_pos[1],
                    p.world_pos[2] + p.half_extents[1],
                );
                let rotation = if p.rotation_x_deg != 0.0 {
                    Mat4::from_rotation_x(p.rotation_x_deg.to_radians())
                } else {
                    Mat4::IDENTITY
                };
                let model = Mat4::from_translation(center)
                    * rotation
                    * Mat4::from_scale(glam::Vec3::new(
                        p.half_extents[0] * 2.0,
                        p.half_extents[1] * 2.0,
                        p.half_extents[2] * 2.0,
                    ));
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
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color,
                    specular_strength: 0.45 + 0.55 * g,
                    specular_power: 32.0,
                };
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
                        ],
                    });
                    self.relic_slot_texture[slot_i] = want_tex;
                }
                self.last_relic_models.push(model);
                self.last_debug_pickables
                    .push(("Relic", model, glam::Vec3::splat(0.5), 0.0));

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
                self.proj.relic_rects
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
                        p.world_pos[0],
                        p.world_pos[1],
                        p.world_pos[2] + p.half_extents[1],
                    );
                    let rot_x = if p.rotation_x_deg != 0.0 {
                        Mat4::from_rotation_x(p.rotation_x_deg.to_radians())
                    } else {
                        Mat4::IDENTITY
                    };
                    let rot_y = if p.rotation_y_deg != 0.0 {
                        Mat4::from_rotation_y(p.rotation_y_deg.to_radians())
                    } else {
                        Mat4::IDENTITY
                    };
                    let model = Mat4::from_translation(center)
                        * rot_y
                        * rot_x
                        * Mat4::from_scale(glam::Vec3::new(
                            p.half_extents[0] * 2.0,
                            p.half_extents[1] * 2.0,
                            p.half_extents[2] * 2.0,
                        ));
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
                    self.proj.pack_rects
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
                let center = pixel_to_world(cx, cy, dh * 0.5);
                let model =
                    Mat4::from_translation(center) * Mat4::from_scale(glam::Vec3::new(dw, dh, dd));
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
            let center = pixel_to_world(c.center_pos[0], c.center_pos[1], c.center_pos[2]);
            let half = glam::Vec3::new(c.extents[0] * 0.5, c.extents[1] * 0.5, c.extents[2] * 0.5);
            let model = Mat4::from_translation(center)
                * Mat4::from_scale(glam::Vec3::new(c.extents[0], c.extents[1], c.extents[2]));
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
                        s.world_pos[0],
                        s.world_pos[1],
                        s.world_pos[2] + s.extents[1] * 0.5,
                    );
                    let model = Mat4::from_translation(center)
                        * Mat4::from_scale(glam::Vec3::new(
                            s.extents[0],
                            s.extents[1],
                            s.extents[2],
                        ));
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
                    self.proj.shrine_rects
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
                &self.tile_sampler,
            ));
        }
        for (slot_i, d) in aux_dish_cmds.iter().enumerate() {
            let center = pixel_to_world(
                d.center_pos[0],
                d.center_pos[1],
                d.center_pos[2] + d.extents[1] * 0.5,
            );
            let model = Mat4::from_translation(center)
                * Mat4::from_scale(glam::Vec3::new(d.extents[0], d.extents[1], d.extents[2]));
            self.aux_dish_instances[slot_i].write_uniform(
                &self.queue,
                view_proj_arr,
                model,
                self.dish_mesh.default_material,
            );
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
            self.proj.aux_dish_rects
                .push((d.pick_id, [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]));
            self.last_aux_dish_aabbs.push((center, half));
        }
        // Append pack projected rects so tooltip anchoring and focus nav
        // pick them up via the existing `aux_dish_rects` path.
        for (rect, pick_id) in &self.proj.pack_rects {
            self.proj.aux_dish_rects.push((*pick_id, *rect));
        }

        // ── Ribbon batches (shop scene) ────────────────────────────────
        // Each textured ribbon uses up to 3 draw slots (top cap, tileable
        // middle, bottom cap) so its length is independent of texture aspect.
        // Untextured (plain) ribbons still use a single slot.
        self.proj.ribbon_rects.clear();
        self.last_ribbon_models.clear();
        self.last_ribbon_batch_slot_counts.clear();
        let mut ribbon_slot_cursor: usize = 0;
        for batch in &ribbon_batches {
            let batch_start = ribbon_slot_cursor;
            for r in batch.iter() {
                if ribbon_slot_cursor >= MAX_RIBBON_SLOTS {
                    break;
                }
                let anchor = pixel_to_world(r.anchor_pos[0], r.anchor_pos[1], r.anchor_pos[2]);
                let eff_w = r.width;
                let eff_l = r.length;
                let depth = eff_w * 0.15;
                let base_transform = Mat4::from_translation(anchor)
                    * Mat4::from_rotation_z(r.rotation_z_deg.to_radians())
                    * Mat4::from_rotation_y(r.rotation_y_deg.to_radians())
                    * Mat4::from_rotation_x(r.rotation_x_deg.to_radians());
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
                    let top_model = base_transform
                        * Mat4::from_scale(glam::Vec3::new(eff_w, cap_h, depth));
                    emit_slot(ribbon_slot_cursor, top_model, Some((idx, 0)));
                    ribbon_slot_cursor += 1;

                    // Middle (stretches to fill remaining length)
                    if mid_h > 0.0 {
                        let mid_model = base_transform
                            * Mat4::from_translation(glam::Vec3::new(0.0, -cap_h, 0.0))
                            * Mat4::from_scale(glam::Vec3::new(eff_w, mid_h, depth));
                        emit_slot(ribbon_slot_cursor, mid_model, Some((idx, 1)));
                        ribbon_slot_cursor += 1;
                    }

                    // Bottom cap
                    let bot_model = base_transform
                        * Mat4::from_translation(glam::Vec3::new(0.0, -(cap_h + mid_h), 0.0))
                        * Mat4::from_scale(glam::Vec3::new(eff_w, cap_h, depth));
                    emit_slot(ribbon_slot_cursor, bot_model, Some((idx, 2)));
                    ribbon_slot_cursor += 1;

                    // For pick-testing, store the full-ribbon model matrix.
                    let full_model = base_transform
                        * Mat4::from_scale(glam::Vec3::new(eff_w, eff_l, depth));
                    self.last_ribbon_models.push(full_model);
                } else {
                    // Untextured (plain) ribbon — single slot, same as before.
                    let model = base_transform
                        * Mat4::from_scale(glam::Vec3::new(eff_w, eff_l, depth));
                    emit_slot(ribbon_slot_cursor, model, None);
                    ribbon_slot_cursor += 1;
                    self.last_ribbon_models.push(model);
                }

                // Project the ribbon's full AABB to screen for tooltip/click.
                let full_model = base_transform
                    * Mat4::from_scale(glam::Vec3::new(eff_w, eff_l, depth));
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
                self.proj.ribbon_rects
                    .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
            }
            self.last_ribbon_batch_slot_counts
                .push(ribbon_slot_cursor - batch_start);
        }
        self.last_ribbon_slot_count = ribbon_slot_cursor;

        // ── Talisman batches (shop scene) ──────────────────────────────
        self.proj.talisman_rects.clear();
        self.last_talisman_models.clear();
        let mut talisman_slot_cursor: usize = 0;
        for batch in &talisman_batches {
            for t in batch.iter() {
                if talisman_slot_cursor >= MAX_TALISMAN_SLOTS {
                    break;
                }
                let slot_i = talisman_slot_cursor;
                talisman_slot_cursor += 1;
                let center = pixel_to_world(t.center_pos[0], t.center_pos[1], t.center_pos[2]);
                // Talisman mesh local extents are (HALF_W, HALF_H, HALF_T) ≈
                // (0.5, 0.7, 0.09); scale so the world-space bounds match
                // the requested extents.
                let sx = t.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                let sy = t.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                let sz = t.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                let model = Mat4::from_translation(center)
                    * Mat4::from_rotation_z(t.rotation_z_deg.to_radians())
                    * Mat4::from_rotation_y(t.rotation_y_deg.to_radians())
                    * Mat4::from_rotation_x(t.rotation_x_deg.to_radians())
                    * Mat4::from_scale(glam::Vec3::new(sx, sy, sz));
                let material = MaterialParams {
                    kind: MaterialKind::Talisman,
                    base_color: t.color,
                    specular_strength: 0.55,
                    specular_power: 48.0,
                };
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
                self.proj.talisman_rects
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
                let center = pixel_to_world(c.world_pos[0], c.world_pos[1], c.world_pos[2]);
                let model = Mat4::from_translation(center)
                    * Mat4::from_rotation_y(c.rotation_y)
                    * Mat4::from_scale(glam::Vec3::new(
                        c.radius * 2.0,
                        c.thickness,
                        c.radius * 2.0,
                    ));
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

        // ── Skeuomorphic gameplay HUD uniform writes (phase 1) ─────────
        //
        // The new HUD meshes (plaque, ofuda, tablets, bowl, peg block, wall
        // stack, dora stand) all share the lit-mesh pipeline. Each gets its
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
        // The plaque mesh's broad face is in the XY plane (normal +Z), but
        // the gameplay camera looks down at the table from `(0, ~h*0.55,
        // ~h*0.95)` — about 28° below horizontal — so a strictly vertical
        // plaque presents nearly edge-on. We tilt the plaque face up about
        // the X axis so its normal lifts toward the camera and the wood
        // reads as a flat panel rather than a thin sliver hovering above
        // the back of the table.
        let plaque_tilt_x = 0.0_f32.to_radians();
        self.proj.plaque_rects.clear();
        for (slot_i, p) in plaque_cmds.iter().enumerate() {
            if slot_i >= MAX_PLAQUE_SLOTS {
                break;
            }
            let center = pixel_to_world(p.center_pos[0], p.center_pos[1], p.center_pos[2]);
            let model = Mat4::from_translation(center)
                * Mat4::from_rotation_y(p.rotation_y_deg.to_radians())
                * Mat4::from_rotation_x(plaque_tilt_x)
                * Mat4::from_scale(glam::Vec3::new(p.extents[0], p.extents[1], p.extents[2]));
            // Engraved two-line decal painted on the +Z face. Empty top
            // *and* empty bottom = no decal needed (the second placard
            // plaque uses this path with no engraved text). Otherwise
            // rasterize once when either line changes and treat the
            // texture as a transparent overlay via `has_decal = true`.
            let has_decal_text = !p.top_text.is_empty() || !p.bot_text.is_empty();
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
                let combined = format!("{}\n{}", p.top_text, p.bot_text);
                let label_hash = tablet_label_hash(&combined, decal_w, decal_h);
                let inst = &mut self.plaque_instances[slot_i];
                if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                    let rgba = crate::render::decal::rasterize_plaque_decal(
                        &p.top_text,
                        &p.bot_text,
                        self.ui_font.as_ref(),
                        decal_w,
                        decal_h,
                    );
                    inst.set_decal(
                        &self.device,
                        &self.queue,
                        &self.lit_mesh_material_layout,
                        &self.tile_sampler,
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
            self.proj.plaque_rects
                .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
            // Plaque local AABB is the unit cube (push_box convention).
            // Slot 0 is the blind plaque, slot 1 is the scoring placard.
            let name = if slot_i == 0 {
                "BlindPlaque"
            } else {
                "ScoringPlacard"
            };
            self.last_debug_pickables
                .push((name, model, glam::Vec3::splat(0.5), 0.0));
        }

        // Ofuda (single instance per cmd). The paper hangs upright on the
        // back wall — no camera-toward tilt — so the wrapped rule decal
        // reads as a posted notice instead of foreshortening into a sliver.
        let ofuda_tilt_x = 0.0_f32.to_radians();
        for (slot_i, p) in ofuda_cmds.iter().enumerate() {
            if slot_i >= MAX_OFUDA_SLOTS {
                break;
            }
            let center = pixel_to_world(p.center_pos[0], p.center_pos[1], p.center_pos[2]);
            let model = Mat4::from_translation(center)
                * Mat4::from_rotation_y(p.rotation_y_deg.to_radians())
                * Mat4::from_rotation_x(ofuda_tilt_x)
                * Mat4::from_scale(glam::Vec3::new(p.extents[0], p.extents[1], p.extents[2]));
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
            self.last_debug_pickables
                .push(("BossOfuda", model, glam::Vec3::splat(0.5), 0.0));
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
                    t.world_pos[0],
                    t.world_pos[1],
                    t.world_pos[2] + t.extents[1] * 0.5 + lift,
                );
                let model = Mat4::from_translation(center)
                    * Mat4::from_scale(glam::Vec3::new(t.extents[0], t.extents[1], t.extents[2]));
                // Active tablets warm up to a champagne tint; dim ones stay
                // bone. The decal pass (phase 2) will paint the engraved name
                // on top via a per-instance albedo texture.
                let base = if t.active {
                    [1.00, 0.92, 0.72, 1.0]
                } else {
                    [0.93, 0.89, 0.78, 1.0]
                };
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: base,
                    specular_strength: 0.30 + 0.20 * t.hover.clamp(0.0, 1.0),
                    specular_power: 32.0,
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
                        &rgba,
                        256,
                        96,
                    );
                    inst.decal_label_hash = label_hash;
                }
                inst.write_uniform_with_decal(&self.queue, view_proj_arr, model, material, true);
                self.proj.yaku_tablet_rects
                    .push(project_unit_cube_rect(model));
                self.last_yaku_tablet_models.push(model);
                self.last_debug_pickables
                    .push(("YakuTablet", model, glam::Vec3::splat(0.5), 0.0));
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
                    t.world_pos[0],
                    t.world_pos[1],
                    t.world_pos[2] + t.extents[1] * 0.5 + lift - press,
                );
                // Tilt the top face toward the camera so the engraved
                // label is readable. The default camera looks down ~25°
                // below horizontal, so rotating +25° around X tips the
                // +Y normal into the view direction.
                let tilt_rad = 25.0_f32.to_radians();
                let model = Mat4::from_translation(center)
                    * Mat4::from_rotation_x(tilt_rad)
                    * Mat4::from_scale(glam::Vec3::new(t.extents[0], t.extents[1], t.extents[2]));
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
                self.proj.wood_tablet_rects
                    .push(project_unit_cube_rect(model));
                self.last_wood_tablet_models.push(model);
                // 0 = sort suit, 1 = sort rank, 2 = yaku journal book.
                // (The play action is now the BronzeMirror, not a wood
                // tablet, so it isn't tracked here.)
                let name = match self.last_wood_tablet_models.len() - 1 {
                    0 => "WoodTablet[SortSuit]",
                    1 => "WoodTablet[SortRank]",
                    2 => "WoodTablet[Journal]",
                    _ => "WoodTablet",
                };
                self.last_debug_pickables
                    .push((name, model, glam::Vec3::splat(0.5), 0.0));
            }
        }

        // Discard bowl (single instance per cmd; gameplay uses 1).
        // The first cmd's `hover` flag drives a smoothed envelope on the
        // renderer (`bowl_hover_anim`) so the lift + camera-tilt animation
        // eases in *and* out instead of snapping. Slot 0 is canonical;
        // additional slots (none in current gameplay) reuse the same
        // envelope, which is fine because the bowl is a singleton.
        if let Some(b0) = bowl_cmds.first() {
            let target = b0.hover.clamp(0.0, 1.0);
            // Exponential ease toward target. Rate ≈ 14 ⇒ ~70 ms time
            // constant — snappy but visibly animated, matches the rest of
            // the HUD's tactile feel.
            let k = 1.0 - (-14.0 * self.frame_dt).exp();
            self.bowl_hover_anim += (target - self.bowl_hover_anim) * k;
        }
        for (slot_i, b) in bowl_cmds.iter().enumerate() {
            if slot_i >= MAX_BOWL_SLOTS {
                break;
            }
            let anim = self.bowl_hover_anim;
            let lift = anim * b.extents[1] * 0.15;
            // Tilt the bowl so its top edge dips toward the camera. The
            // camera looks down at the table from `(0, +Y, +Z)` (~28°
            // below horizontal — see the plaque tilt comment above), so
            // a positive Rx rotation pivots the bowl's +Y axis toward
            // +Z, presenting more of its mouth to the player.
            let tilt = anim * 18.0_f32.to_radians();
            let center = pixel_to_world(
                b.world_pos[0],
                b.world_pos[1],
                b.world_pos[2] + b.extents[1] * 0.5 + lift,
            );
            let model = Mat4::from_translation(center)
                * Mat4::from_rotation_x(tilt)
                * Mat4::from_scale(glam::Vec3::new(b.extents[0], b.extents[1], b.extents[2]));
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
                "DiscardBowl",
                model,
                glam::Vec3::new(BOWL_LOCAL_HALF[0], BOWL_LOCAL_HALF[1], BOWL_LOCAL_HALF[2]),
                BOWL_LOCAL_CENTER_Y,
            ));
        }

        // Bronze mirror (single instance per cmd; gameplay uses 1).
        // Same easing convention as the discard bowl above — see the
        // comment block there for the rationale on the singleton envelope.
        if let Some(m0) = mirror_cmds.first() {
            let target = m0.hover.clamp(0.0, 1.0);
            let k = 1.0 - (-14.0 * self.frame_dt).exp();
            self.mirror_hover_anim += (target - self.mirror_hover_anim) * k;
        }
        for (slot_i, m) in mirror_cmds.iter().enumerate() {
            if slot_i >= MAX_MIRROR_SLOTS {
                break;
            }
            let anim = self.mirror_hover_anim;
            let lift = anim * m.extents[1] * 0.15;
            // Tilt the polished face toward the camera so the cast
            // four-spirit relief catches more candle light at hover.
            // Same Rx sign rationale as the bowl above.
            let tilt = anim * 22.0_f32.to_radians();
            let center = pixel_to_world(
                m.world_pos[0],
                m.world_pos[1],
                m.world_pos[2] + m.extents[1] * 0.5 + lift,
            );
            let model = Mat4::from_translation(center)
                * Mat4::from_rotation_x(tilt)
                * Mat4::from_scale(glam::Vec3::new(m.extents[0], m.extents[1], m.extents[2]));
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
                "BronzeMirror",
                model,
                glam::Vec3::new(
                    MIRROR_LOCAL_HALF[0],
                    MIRROR_LOCAL_HALF[1],
                    MIRROR_LOCAL_HALF[2],
                ),
                MIRROR_LOCAL_CENTER_Y,
            ));
        }

        // Peg blocks: one wood block + N peg cylinders per cmd.
        self.proj.peg_rects = [None, None];
        let mut peg_slot_cursor: usize = 0;
        for (slot_i, p) in peg_block_cmds.iter().enumerate() {
            if slot_i >= MAX_PEG_BLOCK_SLOTS {
                break;
            }
            // The block itself.
            let block_center = pixel_to_world(
                p.world_pos[0],
                p.world_pos[1],
                p.world_pos[2] + p.extents[1] * 0.5,
            );
            let block_model = Mat4::from_translation(block_center)
                * Mat4::from_scale(glam::Vec3::new(p.extents[0], p.extents[1], p.extents[2]));
            self.peg_block_instances[slot_i].write_uniform(
                &self.queue,
                view_proj_arr,
                block_model,
                self.peg_block_mesh.default_material,
            );
            self.last_debug_pickables
                .push(("PegBlock", block_model, glam::Vec3::splat(0.5), 0.0));

            // Pegs are laid on their side and
            // arranged in two horizontal rows on the right end of the
            // plaque: plays on top, discards below.
            let peg_radius = 15.0_f32.max(p.extents[1] * 0.06);
            let peg_length = p.extents[1] * 0.25;
            let peg_step = peg_radius * 4.5;
            let row_gap = peg_radius * 4.5;
            let plays_lift = p.world_pos[2] + row_gap * 0.5;
            let discards_lift = p.world_pos[2] - row_gap * 0.5;

            // Project the two rows (plays on top, discards below) into
            // screen-space rects for hit-testing and focus highlights.
            if slot_i == 0 {
                let plays_w = peg_step * (p.plays_max as f32 - 1.0).max(0.0) + peg_radius * 2.0;
                let disc_w = peg_step * (p.discards_max as f32 - 1.0).max(0.0) + peg_radius * 2.0;
                let row_h = peg_radius * 2.0;
                let plays_center = pixel_to_world(p.world_pos[0], p.world_pos[1], plays_lift);
                let discards_center = pixel_to_world(p.world_pos[0], p.world_pos[1], discards_lift);
                let plays_scale = glam::Vec3::new(plays_w, row_h, p.extents[2]);
                let disc_scale = glam::Vec3::new(disc_w, row_h, p.extents[2]);
                let plays_model =
                    Mat4::from_translation(plays_center) * Mat4::from_scale(plays_scale);
                let discards_model =
                    Mat4::from_translation(discards_center) * Mat4::from_scale(disc_scale);
                self.proj.peg_rects[0] = Some(project_unit_cube_rect(plays_model));
                self.proj.peg_rects[1] = Some(project_unit_cube_rect(discards_model));
            }

            let plays_color: [f32; 4] = [0.42, 0.82, 0.55, 1.0];
            let discards_color: [f32; 4] = [0.96, 0.72, 0.28, 1.0];

            let draw_row = |slot_cursor: &mut usize,
                            peg_instances: &mut [LitMeshInstance],
                            queue: &wgpu::Queue,
                            start_x: f32,
                            lift: f32,
                            count: u32,
                            max_count: u32,
                            color: [f32; 4]| {
                if max_count == 0 {
                    return;
                }
                let n = count.min(max_count) as usize;
                for k in 0..n {
                    if *slot_cursor >= MAX_PEG_SLOTS {
                        return;
                    }
                    let px = start_x + peg_step * k as f32;
                    let center = pixel_to_world(px, p.world_pos[1], lift);
                    let model = Mat4::from_translation(center)
                        * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
                        * Mat4::from_rotation_y(-std::f32::consts::FRAC_PI_2)
                        * Mat4::from_scale(glam::Vec3::new(
                            peg_radius * 2.0,
                            peg_length,
                            peg_radius * 2.0,
                        ));
                    let material = MaterialParams {
                        kind: MaterialKind::Plain,
                        base_color: color,
                        specular_strength: 0.45,
                        specular_power: 56.0,
                    };
                    peg_instances[*slot_cursor].write_uniform(
                        queue,
                        view_proj_arr,
                        model,
                        material,
                    );
                    *slot_cursor += 1;
                }
            };

            // Center both rows on the block's x position.
            let max_pegs = p.plays_max.max(p.discards_max) as f32;
            let total_row_w = peg_step * (max_pegs - 1.0).max(0.0);
            let row_start_x = p.world_pos[0] - total_row_w * 0.5;
            draw_row(
                &mut peg_slot_cursor,
                &mut self.peg_instances,
                &self.queue,
                row_start_x,
                plays_lift,
                p.plays_left,
                p.plays_max,
                plays_color,
            );
            draw_row(
                &mut peg_slot_cursor,
                &mut self.peg_instances,
                &self.queue,
                row_start_x,
                discards_lift,
                p.discards_left,
                p.discards_max,
                discards_color,
            );
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
                let center = pixel_to_world(px, py, pz);
                let model = Mat4::from_translation(center)
                    * Mat4::from_scale(glam::Vec3::new(tile_w, tile_h, tile_d));
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
                self.last_debug_pickables
                    .push(("WallTile", model, glam::Vec3::splat(0.5), 0.0));
                wall_tile_slot_cursor += 1;
            }
        }

        // Dora stands (single instance per cmd).
        for (slot_i, d) in dora_stand_cmds.iter().enumerate() {
            if slot_i >= MAX_DORA_STAND_SLOTS {
                break;
            }
            let center = pixel_to_world(
                d.world_pos[0],
                d.world_pos[1],
                d.world_pos[2] + d.extents[1] * 0.5,
            );
            let model = Mat4::from_translation(center)
                * Mat4::from_scale(glam::Vec3::new(d.extents[0], d.extents[1], d.extents[2]));
            self.dora_stand_instances[slot_i].write_uniform(
                &self.queue,
                view_proj_arr,
                model,
                self.dora_stand_mesh.default_material,
            );
            self.last_debug_pickables
                .push(("DoraStand", model, glam::Vec3::splat(0.5), 0.0));
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
                let center = pixel_to_world(t.world_pos[0], t.world_pos[1], t.world_pos[2]);
                let model = Mat4::from_translation(center)
                    * Mat4::from_scale(glam::Vec3::new(
                        t.extents[0] * pulse_scale,
                        t.extents[1] * pulse_scale,
                        t.extents[2] * pulse_scale,
                    ));
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
                let name = match t.kind {
                    CascadeTokenKind::Chips => "CascadeToken[Chips]",
                    CascadeTokenKind::Mult => "CascadeToken[Mult]",
                };
                self.last_debug_pickables
                    .push((name, model, glam::Vec3::splat(0.5), 0.0));
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
                let center = pixel_to_world(b.world_pos[0], b.world_pos[1], b.world_pos[2]);
                let model = Mat4::from_translation(center)
                    * Mat4::from_rotation_y(b.rotation[1])
                    * Mat4::from_rotation_x(b.rotation[0])
                    * Mat4::from_rotation_z(b.rotation[2])
                    * Mat4::from_scale(glam::Vec3::new(b.extents[0], b.extents[1], b.extents[2]));
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
                self.last_debug_pickables
                    .push(("FallingBone", model, glam::Vec3::splat(0.5), 0.0));
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
                let center = pixel_to_world(g.world_pos[0], g.world_pos[1], g.world_pos[2]);
                let model = Mat4::from_translation(center)
                    * Mat4::from_rotation_y(g.rotation_y)
                    * Mat4::from_rotation_x(-std::f32::consts::PI + g.rotation_x)
                    * Mat4::from_scale(glam::Vec3::splat(g.scale));
                // Pack the emissive boost into specular_strength so the
                // shader brightens the popup without needing a new uniform
                // field. The lit_mesh shader treats specular_strength
                // additively, which doubles as a poor-man's emissive on
                // the Plain material kind.
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: g.color,
                    specular_strength: 0.45 + 0.55 * g.emissive.clamp(0.0, 1.0),
                    specular_power: 64.0,
                };
                self.extruded_glyph_instances[slot_i].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    material,
                );
                self.last_debug_pickables
                    .push(("ScorePopup", model, glam::Vec3::splat(0.5), 0.0));
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

        // ── Volumetric smoke setup (camera, bounds, plumes, cursor) ─────
        // Done before the encoder is created so the per-tile impulses
        // queued during the tile-loop above are still pending. The grid
        // is sized to comfortably bracket the table area in world units.
        if let Some(ref mut fluid) = self.fluid {
            // Grid bounds: a box roughly enclosing the table + headroom for
            // candle plumes. Matches the world space defined by pixel_to_world
            // (table at y=0, x ∈ ±w/2, z ∈ ±h/2).
            let half_w = w * 0.75;
            let half_z = h * 0.75;
            let grid_min = glam::Vec3::new(-half_w, -12.0, -half_z);
            let grid_max = glam::Vec3::new(half_w, h * 0.75, half_z);
            fluid.set_grid_bounds(grid_min, grid_max);

            // Candle plume impulses — walk this frame's draw commands and
            // emit a steady upward velocity + density at every wick tip.
            for cmd in frame.cmds.iter() {
                if let DrawCmd::CandleBatch(placements) = cmd {
                    for p in placements.iter() {
                        let tip = pixel_to_world(
                            p.world_pos[0],
                            p.world_pos[1],
                            crate::render::candle_mesh::WICK_TIP_Y * p.scale * p.height_scale,
                        );
                        // Soft hot point source: barely any kinetic launch
                        // velocity — buoyancy in the advect step does the
                        // lifting, and the height-attenuated curl field
                        // turns the rising column into a spreading plume.
                        // Pumping a hard upward velocity here used to
                        // produce a tight rocket pillar that the curl
                        // couldn't bend; the new injection is dominated
                        // by density (heat) rather than momentum.
                        fluid.inject_impulse(tip, glam::Vec3::new(0.0, 6.0, 0.0), 18.0, 0.10);
                    }
                }
            }

            // Scene-driven wind gusts. Scenes (currently gameplay) push these
            // when they want a deliberate, time-shaped breath of wind on the
            // smoke — e.g. blowing the post-deal smoke off the hand strip a
            // few seconds after dealing. Coordinates are layout pixels; we
            // run them through the same `pixel_to_world` projection used by
            // candle plumes so the gust lands on the table plane.
            for g in frame.wind_gusts.iter() {
                let pos = pixel_to_world(g.center_px.0, g.center_px.1, g.lift);
                fluid.inject_impulse(
                    pos,
                    glam::Vec3::new(g.velocity[0], g.velocity[1], g.velocity[2]),
                    g.radius,
                    g.density,
                );
            }

            // Cursor → table-plane impulse. Project the screen cursor with
            // the inverse of the gameplay view-projection, intersect the
            // y=0 plane, and inject a soft "wind" puff in the direction of
            // motion. Provides reactive disturbance for moving the mouse.
            if let Some((cx, cy)) = frame.cursor_pos {
                let inv_vp = view_proj.inverse();
                let nx = (cx / w) * 2.0 - 1.0;
                let ny = 1.0 - (cy / h) * 2.0;
                let near = inv_vp * glam::Vec4::new(nx, ny, 0.0, 1.0);
                let far = inv_vp * glam::Vec4::new(nx, ny, 1.0, 1.0);
                let near3 = glam::Vec3::new(near.x / near.w, near.y / near.w, near.z / near.w);
                let far3 = glam::Vec3::new(far.x / far.w, far.y / far.w, far.z / far.w);
                let dir = (far3 - near3).normalize_or_zero();
                if dir.y.abs() > 1e-4 {
                    // Intersect with y = 5 (just above the table surface).
                    let plane_y = 5.0;
                    let t = (plane_y - near3.y) / dir.y;
                    if t > 0.0 {
                        let hit = near3 + dir * t;
                        if let Some(prev) = self.prev_cursor_world {
                            let delta = hit - prev;
                            let speed = delta.length();
                            // Scale the cursor puff with window size so it
                            // reads at a similar visual proportion at every
                            // resolution. 1 px ≈ 1 world unit here, so a
                            // fixed radius would shrink to a pinprick on
                            // large displays. Baseline is 1080p — matches
                            // how `cam_pos` and the fluid grid bounds scale.
                            let win_scale = (h / 1080.0).max(0.5);
                            let speed_threshold = 1.0 * win_scale;
                            if speed > speed_threshold {
                                let inv_dt = 1.0 / dt.max(1.0 / 120.0);
                                // Density needs to be in the same regime as
                                // the candle injection (~0.1) so the volume
                                // raymarcher actually picks it up. The old
                                // `speed * 0.018` value made each puff one
                                // or two orders of magnitude fainter than
                                // the candle plumes — invisible against
                                // the now-properly-rising columns. Use a
                                // saturating curve so fast cursor flicks
                                // don't blow out, plus a small upward kick
                                // so the puff actually lifts off the table
                                // instead of sitting at hit point until
                                // buoyancy slowly catches up.
                                let puff_density = (speed * 0.05).min(0.18);
                                fluid.inject_impulse(
                                    hit + glam::Vec3::new(0.0, 4.0 * win_scale, 0.0),
                                    delta * inv_dt * 0.35 + glam::Vec3::new(0.0, 12.0, 0.0),
                                    28.0 * win_scale,
                                    puff_density,
                                );
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
            fluid.upload_camera_uniform(&self.queue, view_proj, cam_pos, smoke_intensity);
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
                    placement.world_pos[0],
                    placement.world_pos[1],
                    placement.world_pos[2],
                );
                let s = placement.scale;
                let model = Mat4::from_translation(base)
                    * Mat4::from_scale(glam::Vec3::new(s, s * placement.height_scale, s));
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
                        p.world_pos[0],
                        p.world_pos[1],
                        p.world_pos[2] + p.half_extents[1],
                    );
                    let model = Mat4::from_translation(center)
                        * Mat4::from_scale(glam::Vec3::new(
                            p.half_extents[0] * 2.0,
                            p.half_extents[1] * 2.0,
                            p.half_extents[2] * 2.0,
                        ));
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
            let center = pixel_to_world(c.center_pos[0], c.center_pos[1], c.center_pos[2]);
            let model = Mat4::from_translation(center)
                * Mat4::from_scale(glam::Vec3::new(c.extents[0], c.extents[1], c.extents[2]));
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
                        s.world_pos[0],
                        s.world_pos[1],
                        s.world_pos[2] + s.extents[1] * 0.5,
                    );
                    let model = Mat4::from_translation(center)
                        * Mat4::from_scale(glam::Vec3::new(
                            s.extents[0],
                            s.extents[1],
                            s.extents[2],
                        ));
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
                d.center_pos[0],
                d.center_pos[1],
                d.center_pos[2] + d.extents[1] * 0.5,
            );
            let model = Mat4::from_translation(center)
                * Mat4::from_scale(glam::Vec3::new(d.extents[0], d.extents[1], d.extents[2]));
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
                        pixel_to_world(r.anchor_pos[0], r.anchor_pos[1], r.anchor_pos[2]);
                    let eff_w = r.width;
                    let eff_l = r.length;
                    let depth = eff_w * 0.15;
                    let base_transform = Mat4::from_translation(anchor)
                        * Mat4::from_rotation_z(r.rotation_z_deg.to_radians())
                        * Mat4::from_rotation_y(r.rotation_y_deg.to_radians())
                        * Mat4::from_rotation_x(r.rotation_x_deg.to_radians());

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
                        let top_model = base_transform
                            * Mat4::from_scale(glam::Vec3::new(eff_w, cap_h, depth));
                        self.ribbon_instances[ribbon_shadow_cursor]
                            .write_shadow_uniform(&self.queue, light_view_proj_arr, top_model);
                        ribbon_shadow_cursor += 1;
                        // Middle
                        if mid_h > 0.0 {
                            let mid_model = base_transform
                                * Mat4::from_translation(glam::Vec3::new(0.0, -cap_h, 0.0))
                                * Mat4::from_scale(glam::Vec3::new(eff_w, mid_h, depth));
                            self.ribbon_instances[ribbon_shadow_cursor]
                                .write_shadow_uniform(
                                    &self.queue,
                                    light_view_proj_arr,
                                    mid_model,
                                );
                            ribbon_shadow_cursor += 1;
                        }
                        // Bottom cap
                        let bot_model = base_transform
                            * Mat4::from_translation(glam::Vec3::new(
                                0.0,
                                -(cap_h + mid_h),
                                0.0,
                            ))
                            * Mat4::from_scale(glam::Vec3::new(eff_w, cap_h, depth));
                        self.ribbon_instances[ribbon_shadow_cursor]
                            .write_shadow_uniform(&self.queue, light_view_proj_arr, bot_model);
                        ribbon_shadow_cursor += 1;
                    } else {
                        let model = base_transform
                            * Mat4::from_scale(glam::Vec3::new(eff_w, eff_l, depth));
                        self.ribbon_instances[ribbon_shadow_cursor]
                            .write_shadow_uniform(&self.queue, light_view_proj_arr, model);
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
                    let center = pixel_to_world(t.center_pos[0], t.center_pos[1], t.center_pos[2]);
                    let sx = t.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                    let sy = t.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                    let sz = t.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                    let model = Mat4::from_translation(center)
                        * Mat4::from_rotation_z(t.rotation_z_deg.to_radians())
                        * Mat4::from_rotation_y(t.rotation_y_deg.to_radians())
                        * Mat4::from_rotation_x(t.rotation_x_deg.to_radians())
                        * Mat4::from_scale(glam::Vec3::new(sx, sy, sz));
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
                    let center = pixel_to_world(c.world_pos[0], c.world_pos[1], c.world_pos[2]);
                    let model = Mat4::from_translation(center)
                        * Mat4::from_rotation_y(c.rotation_y)
                        * Mat4::from_scale(glam::Vec3::new(
                            c.radius * 2.0,
                            c.thickness,
                            c.radius * 2.0,
                        ));
                    self.coin_instances[slot_i].write_shadow_uniform(
                        &self.queue,
                        light_view_proj_arr,
                        model,
                    );
                }
            }
        }
        if needs_dish {
            if let Some((lo_x, lo_y, hi_x, hi_y)) = dish_bounds {
                let cx = (lo_x + hi_x) * 0.5;
                let cy = (lo_y + hi_y) * 0.5;
                let dw = (hi_x - lo_x).max(40.0);
                let dd = (hi_y - lo_y).max(28.0);
                let dh = 10.0_f32;
                let center = pixel_to_world(cx, cy, dh * 0.5);
                let model =
                    Mat4::from_translation(center) * Mat4::from_scale(glam::Vec3::new(dw, dh, dd));
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

            let mut slot_cursor = 0usize;
            for batch in &showcase_tile_batches {
                for p in batch.iter() {
                    if slot_cursor >= MAX_SHOWCASE_TILE_SLOTS {
                        break;
                    }
                    let wanted_id = (p.tile.suit, p.tile.rank, p.tile.enhancement);
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
                    let center = pixel_to_world(p.center_pos[0], p.center_pos[1], p.center_pos[2]);
                    let tile_short_px = p.size_px * 0.85;
                    let tile_long_px = tile_short_px * tile_preset.face_long_ratio();
                    let tile_thickness_px = tile_short_px * tile_preset.thickness_ratio();
                    let scale = glam::Vec3::new(
                        tile_long_px / LOCAL_X_EXTENT,
                        tile_thickness_px / LOCAL_Y_EXTENT,
                        tile_short_px / LOCAL_Z_EXTENT,
                    ) * p.scale;

                    let rotation = Mat4::from_euler(
                        glam::EulerRot::XYZ,
                        p.rotation[0],
                        p.rotation[1],
                        p.rotation[2],
                    );

                    let model = Mat4::from_translation(center)
                        * rotation
                        * tile_basis
                        * Mat4::from_scale(scale);

                    let stg = &self.showcase_tiles[slot_cursor];
                    let mut sc_bcf = self.tile_base_color_factor;
                    sc_bcf[2] = p.tile.enhancement.map_or(0.0, |e| e.shader_id());
                    self.queue.write_buffer(
                        &stg.uniform_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_proj: view_proj_arr,
                            model: model.to_cols_array(),
                            base_color_factor: sc_bcf,
                        }),
                    );
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

            // Pick the dominant horizontal axis (X or Z; Y is up) by
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
            fluid.step(&mut encoder, &self.queue, step_dt, smoke_intensity);
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
                let total_relics = relic_batches
                    .iter()
                    .map(|b| b.len())
                    .sum::<usize>()
                    .min(MAX_RELIC_SLOTS);
                if total_relics > 0 {
                    shadow_pass.set_vertex_buffer(0, self.relic_box_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.relic_box_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..total_relics {
                        let Some(inst) = self.relic_instances.get(slot_i) else {
                            break;
                        };
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..self.relic_box_mesh.index_count, 0, 0..1);
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

            // Auxiliary dishes (shop).
            {
                let n_aux = aux_dish_cmds.len();
                if n_aux > 0 {
                    shadow_pass.set_vertex_buffer(0, self.dish_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.dish_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..n_aux {
                        shadow_pass.set_bind_group(
                            0,
                            &self.aux_dish_instances[slot_i].shadow_bind_group,
                            &[],
                        );
                        shadow_pass.draw_indexed(0..self.dish_mesh.index_count, 0, 0..1);
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

            // Coins (shop). Skipped for shadow correctness — coins are
            // small and a pile of 30+ shadow draws is wasteful.
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
                    pass.set_vertex_buffer(0, self.relic_box_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.relic_box_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    // Compute the global slot offset for this batch from
                    // the cumulative lengths of preceding RelicBatch cmds.
                    let mut start_slot = 0usize;
                    for prev in 0..*batch_idx {
                        start_slot += relic_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.relic_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.relic_box_mesh.index_count, 0, 0..1);
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
                RenderOp::PackBatch(batch_idx) => {
                    let batch = pack_batches[*batch_idx];
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
                        start_slot += pack_batches[prev].len();
                    }
                    for (i, _) in batch.iter().enumerate() {
                        let slot_i = start_slot + i;
                        let Some(inst) = self.pack_instances.get(slot_i) else {
                            break;
                        };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.draw_indexed(0..self.relic_box_mesh.index_count, 0, 0..1);
                    }
                }
                RenderOp::DishExplicit(idx) => {
                    if *idx < self.aux_dish_instances.len() {
                        pass.set_pipeline(&self.lit_mesh_pipeline);
                        pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                        pass.set_bind_group(0, &self.aux_dish_instances[*idx].bind_group, &[]);
                        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.dish_mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            self.dish_mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..self.dish_mesh.index_count, 0, 0..1);
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
                RenderOp::PegBlock(slot_i) => {
                    // Skip the wooden block mesh — only draw the peg
                    // cylinders, which now float beside the plaque.

                    // Peg cylinders that belong to this block.
                    // Phase 1 wires the geometry through using the global peg
                    // slot pool — the per-block slice math will land alongside
                    // the gameplay-side push in phase 4.
                    let block = peg_block_cmds[*slot_i];
                    let mut start_slot = 0usize;
                    for prev in 0..*slot_i {
                        let pb = peg_block_cmds[prev];
                        start_slot += (pb.plays_left.min(pb.plays_max)
                            + pb.discards_left.min(pb.discards_max))
                            as usize;
                    }
                    let n = (block.plays_left.min(block.plays_max)
                        + block.discards_left.min(block.discards_max))
                        as usize;
                    if n > 0 {
                        pass.set_pipeline(&self.lit_mesh_pipeline);
                        pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.coin_mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            self.coin_mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        for k in 0..n {
                            let s = start_slot + k;
                            let Some(inst) = self.peg_instances.get(s) else {
                                break;
                            };
                            pass.set_bind_group(0, &inst.bind_group, &[]);
                            pass.draw_indexed(0..self.coin_mesh.index_count, 0, 0..1);
                        }
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
                RenderOp::DoraStand(slot_i) => {
                    let Some(inst) = self.dora_stand_instances.get(*slot_i) else {
                        return;
                    };
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(0, &inst.bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.dora_stand_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.dora_stand_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..self.dora_stand_mesh.index_count, 0, 0..1);
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
                            // string) — skip this draw silently.
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
                RenderOp::ShowcaseTileBatch(batch_idx) => {
                    if !self.tile_primitives.is_empty() {
                        let batch = showcase_tile_batches[*batch_idx];
                        if !batch.is_empty() {
                            pass.set_pipeline(&self.tile_pipeline);
                            pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                            pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                            let mut start_slot = 0usize;
                            for prev in 0..*batch_idx {
                                start_slot += showcase_tile_batches[prev].len();
                            }
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
                RenderOp::HandTileBackdrop => {
                    if let Some(ref lbb) = light_beam_buffer {
                        pass.set_pipeline(&self.light_beam_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, lbb.slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..light_beams.len() as u32);
                    }
                    // Selected-tile additive glow halos (drawn first
                    // so the warm light spills out from behind the
                    // tile silhouette). Independent of `tile_quads`,
                    // which is empty now that the old flat halo is
                    // gone — earlier this draw was nested inside the
                    // tile_instance_buffer check and was being
                    // silently skipped on every frame.
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
                    if let Some(ref tib) = tile_instance_buffer {
                        // Halo/selection backdrop quads (drawn before the
                        // 3D tile mesh so the tile sits on top of them).
                        pass.set_pipeline(&self.tile_quad_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, tib.slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..tile_quads.len() as u32);
                    }
                    // 3D hand tiles: one draw per (tile, primitive).
                    // Tiles in the GLB have multiple material slots
                    // (e.g. ivory face + bamboo body); draw each.
                    if !self.tile_primitives.is_empty() {
                        // Point lights (group 1) and shadow sampling
                        // (group 2) are the same for every tile this
                        // frame — bind once outside the loop.
                        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);

                        // Pass A: gold outline shells for selected
                        // tiles. Drawn FIRST so the regular tile mesh
                        // (drawn next) overwrites the interior of the
                        // shell, leaving a thin metallic rim around
                        // each selected tile that catches candlelight.
                        pass.set_pipeline(&self.tile_outline_pipeline);
                        for (i, _) in &tile_3d_rects {
                            if !selected.get(*i).copied().unwrap_or(false) {
                                continue;
                            }
                            let Some(htg) = self.hand_tiles.get(*i) else {
                                continue;
                            };
                            for (pi, prim) in self.tile_primitives.iter().enumerate() {
                                let Some(bg) = htg.outline_bind_groups.get(pi) else {
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

                        // Pass B: the regular textured tile meshes.
                        pass.set_pipeline(&self.tile_pipeline);
                        for (i, _) in &tile_3d_rects {
                            let Some(htg) = self.hand_tiles.get(*i) else {
                                continue;
                            };
                            for (pi, prim) in self.tile_primitives.iter().enumerate() {
                                let Some(bg) = htg.bind_groups.get(pi) else {
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
                RenderOp::HandTileFaces => {
                    for td in &hand_face_draws {
                        pass.set_pipeline(&self.text_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &td.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, td.inst_buf.slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..1);
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
                    let rd = &relic_draws[*idx];
                    if let Some(rtex) = self.relic_textures.get(&rd.relic_id) {
                        pass.set_pipeline(&self.image_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &rtex.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, rd.inst_buf.slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..1);
                    }
                }
            }
        }; // end process_op closure

        // ── Pass A: clear + draw everything that lives behind the smoke ──
        {
            let main_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::Main);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
                texture: &surface_frame.texture,
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
                    let smoke_off_ts = self
                        .gpu_profiler
                        .pass_writes(crate::render::gpu_profiler::PassSlot::SmokeOffscreen);
                    fluid.render_offscreen(
                        &mut encoder,
                        &self.globals_bind_group,
                        scissor,
                        smoke_off_ts,
                    );
                }
            }

            let smoke_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::PostSmoke);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post-smoke-pass"),
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
                timestamp_writes: smoke_ts,
                multiview_mask: None,
            });
            for op in &ops[split..] {
                if matches!(op, RenderOp::TextDraw(_)) {
                    continue;
                }
                process_op(&mut pass, op);
            }
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
