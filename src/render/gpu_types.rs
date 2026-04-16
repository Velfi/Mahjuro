//! GPU type definitions extracted from `wgpu_renderer.rs`.
//! Not yet wired in — `wgpu_renderer.rs` still defines its own copies.
//! Suppress dead-code warnings until the migration is complete.
#![allow(dead_code)]

use glam::Mat4;

use crate::core::relic::RelicId;
use crate::core::tile::{Suit, TileEnhancement};
use crate::render::world_space::pixel_to_world;
use crate::scenes::BackgroundId;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Globals {
    pub screen: [f32; 2],
    pub time: f32,
    pub gamma: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct CameraUniform {
    pub view_proj: [f32; 16],
    pub model: [f32; 16],
    pub base_color_factor: [f32; 4],
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
pub(crate) fn tablet_label_hash(label: &str, w: u32, h: u32) -> u64 {
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
pub(crate) struct TileOccluderGpu {
    /// xyz = world-space AABB center, w = unused.
    pub center: [f32; 4],
    /// xyz = world-space AABB half-extents, w = unused.
    pub half_extents: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TileOccludersBuf {
    /// `count.x` = number of active occluders; rest is std140 padding.
    pub count: [u32; 4],
    pub boxes: [TileOccluderGpu; MAX_TILE_OCCLUDERS],
}

impl TileOccludersBuf {
    pub fn empty() -> Self {
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
pub(crate) struct PointLightGpu {
    /// xyz = world-space position, w = radius.
    pub pos: [f32; 4],
    /// rgb = colour, a = intensity.
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PointLightsBuf {
    /// `count.x` = number of active lights; rest is std140 padding.
    pub count: [u32; 4],
    /// Frame-wide extras shared with shaders that bind this buffer:
    /// `extras.x` = display gamma (used to gamma-correct 3D fragments
    /// that don't have access to the screen-space `Globals` uniform).
    /// `extras.y` = wall-clock time in seconds (used by `MaterialKind::Water`
    /// to scroll the river surface and animate foam crests).
    /// `extras.z` = candle flame height in world units (for the volumetric
    /// lightbake flame emission envelope).
    /// `extras.w` reserved.
    pub extras: [f32; 4],
    pub lights: [PointLightGpu; MAX_POINT_LIGHTS],
}

impl PointLightsBuf {
    /// Build the std140 light buffer via [`pixel_to_world`] (Z-up world).
    pub fn from_lights(
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
pub(crate) struct TilePrimitiveGpu {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub albedo_texture: wgpu::Texture,
    pub albedo_view: wgpu::TextureView,
    pub base_color_factor: [f32; 4],
}

/// A relic icon to draw as a textured quad at a screen-space rect.
pub struct RelicIcon {
    /// Position in screen pixels: [x, y, w, h].
    pub rect: [f32; 4],
    /// Which relic image to display.
    pub relic_id: RelicId,
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
pub(crate) struct HandTileGpu {
    /// Written every frame with view_proj + model + base_color_factor.
    pub uniform_buffer: wgpu::Buffer,
    /// One bind group per tile-mesh primitive.  Each binds the per-tile uniform
    /// + per-tile decal + that primitive's own albedo texture.
    pub bind_groups: Vec<wgpu::BindGroup>,
    /// Companion uniform buffer for the gold-metal outline shell. Written
    /// every frame the tile is *selected* with an inflated model matrix
    /// (uniform 1.06× scale around the tile center). Always allocated so
    /// the bind group can stay constant for the lifetime of the tile.
    pub outline_uniform_buffer: wgpu::Buffer,
    /// Bind groups that point at `outline_uniform_buffer` instead of the
    /// regular one. Same layout as `bind_groups`.
    pub outline_bind_groups: Vec<wgpu::BindGroup>,
    /// Per-tile shadow caster uniform (light_view_proj * model). Written
    /// every frame in lockstep with `uniform_buffer` and consumed by the
    /// shadow pre-pass via `shadow_bind_group`.
    pub shadow_uniform_buffer: wgpu::Buffer,
    pub shadow_bind_group: wgpu::BindGroup,
    /// Cached to skip re-rasterisation when the tile hasn't changed.
    pub tile_id: (Suit, u8, Option<TileEnhancement>, bool),
    /// Main label (number or name) for the tile face.
    pub symbol: String,
    /// Emoji suit indicator rendered below the main label.
    pub suit_emoji: String,
    /// Suit colour for rendering the symbol (RGBA, linear).
    pub suit_color: [f32; 4],
    /// Kept alive so the GPU texture is not freed while bind_group references it.
    #[allow(dead_code)]
    pub decal_texture: wgpu::Texture,
}

/// Simplified `HandTileGpu` for showcase tiles (pack celebration, etc.).
/// No outline buffers, no text metadata — display only.
pub(crate) struct ShowcaseTileGpu {
    pub uniform_buffer: wgpu::Buffer,
    pub bind_groups: Vec<wgpu::BindGroup>,
    pub shadow_uniform_buffer: wgpu::Buffer,
    pub shadow_bind_group: wgpu::BindGroup,
    /// Cache key to skip re-rasterisation when the tile hasn't changed.
    pub tile_id: (Suit, u8, Option<TileEnhancement>, bool),
    #[allow(dead_code)]
    pub decal_texture: wgpu::Texture,
}

pub(crate) const MAX_SHOWCASE_TILE_SLOTS: usize = 160;

// Tile-mesh local extents (after `normalize_mesh` in tile_glb.rs):
//   local X — long face axis  (extent ~1.000) → table-Z (front-back)
//   local Y — thickness        (extent ~0.424) → world Y (up off table)
//   local Z — short face axis  (extent ~0.734) → table-X (left-right)
pub(crate) const LOCAL_X_EXTENT: f32 = 1.000;
pub(crate) const LOCAL_Y_EXTENT: f32 = 0.424;
pub(crate) const LOCAL_Z_EXTENT: f32 = 0.734;

/// Camera state captured at the end of a frame, for unprojecting cursor
/// positions into world-space rays in `pick_hand_tile`.
#[derive(Clone, Copy)]
pub(crate) struct PickCamera {
    pub inv_view_proj: Mat4,
    pub viewport_w: f32,
    pub viewport_h: f32,
}

/// A tile animating away from the hand. Two-phase trajectory: an
/// **Arcing** phase that throws the tile from its hand slot down into the
/// discard river, followed by a **Drifting** phase where the tile rides
/// the current along the channel and fades out. The split is what reads
/// as "throw the tile away" vs the previous fly-off-the-table arc.
pub(crate) struct DepartingTile {
    /// Visual identity for rendering.
    pub symbol: String,
    pub suit_emoji: String,
    pub suit_color: [f32; 4],
    /// Screen-space rect at the moment of departure (top-left + size).
    pub start_rect: [f32; 4],
    /// Splash point — center of the river rect at spawn time, with a
    /// small per-tile jitter so multiple tiles land at slightly different
    /// spots instead of stacking pixel-perfect.
    pub river_target: (f32, f32),
    /// Pixel direction the tile drifts after splashing. Currently
    /// hard-coded to +X (the river flows left-to-right in screen space).
    /// Stored as a unit vector so the render path can change the river
    /// orientation without touching the simulation.
    pub drift_dir: (f32, f32),
    /// Per-tile drift speed in pixels/sec.
    pub drift_speed: f32,
    /// Phase 1 duration (seconds) — how long the arc-into-river takes.
    pub arc_dur: f32,
    /// Phase 2 duration (seconds) — how long the tile drifts before
    /// fading out. The total visible lifetime is `arc_dur + drift_dur`.
    pub drift_dur: f32,
    /// Seconds elapsed since departure started.
    pub elapsed: f32,
    /// Total lifetime convenience field — equals `arc_dur + drift_dur`.
    /// Kept so the existing `retain(|t| t.elapsed < t.lifetime)` cull and
    /// the gameplay-scene refill timer (which reads
    /// `cascade_tuning.depart_lifetime_ms`) keep working.
    pub lifetime: f32,
}

// ── Post-struct types (lines 806-939) ────────────────────────────────────

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
/// `ZodiacBatch` cmds). Each textured ribbon uses up to 3 slots (top/mid/bot).
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
/// Maximum number of cascade scoring tokens per frame across all batches.
/// Structure HUD can show up to 5 chip-tier + 4 mult-tier bones; the modifier
/// strip adds 2 more during an active cascade.
pub const MAX_CASCADE_TOKEN_SLOTS: usize = 32;
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
pub(crate) struct RelicTextureGpu {
    #[allow(dead_code)]
    pub texture: wgpu::Texture,
    /// Bind group for the 2D image pipeline (collection screen).
    pub bind_group: wgpu::BindGroup,
    /// Texture view for binding into lit-mesh material bind groups (3D boxes).
    pub view: wgpu::TextureView,
    /// Owned linear height/relief when uploaded separately; `None` when `relief_view` aliases shared defaults.
    #[allow(dead_code)]
    pub relief_texture: Option<wgpu::Texture>,
    /// Linear grayscale relief (`source/*_height.png`); bound at lit-mesh `relief_tex`.
    pub relief_view: wgpu::TextureView,
}

/// Decoded relic image data sent from the background loader thread.
pub(crate) struct DecodedRelicImage {
    pub id: RelicId,
    pub name: &'static str,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub mesh_rgba: Option<Vec<u8>>,
    pub mesh_width: u32,
    pub mesh_height: u32,
    /// Linear RGBA relief (same UV space as albedo); 1×1 mid-gray when height asset is missing.
    pub relief_rgba: Vec<u8>,
    pub relief_width: u32,
    pub relief_height: u32,
}

/// Pre-loaded background texture + bind group for the image pipeline.
pub(crate) struct BackgroundTextureGpu {
    #[allow(dead_code)]
    pub texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
}

/// Decoded background image data sent from the background loader thread.
pub(crate) struct DecodedBackgroundImage {
    pub id: BackgroundId,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
