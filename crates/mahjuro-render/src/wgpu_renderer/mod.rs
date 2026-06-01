//! WGPU: depth-tested 3D tile meshes for the hand + 2D UI quads on top.

mod embedded_wgsl;
mod frame_pool;
mod boot_splash;
pub mod loading_screen;
mod init;
mod init_deferred;
mod init_phases;
pub(crate) mod resources;

#[cfg(feature = "windowed")]
pub fn run_vulkan_wsi_probe_with_window(window: sdl3::video::Window) -> anyhow::Result<()> {
    init_phases::early_gpu_and_depth(targets::TargetInit::Windowed {
        window,
        hdr_enabled: false,
    })?;
    Ok(())
}
mod runtime;
mod room_gpu_load;
mod showcase;

mod constants;
mod hash_util;
mod internal_slots;
mod layout_instances;
mod lighting_buffers;
mod moon;
mod picking_types;
mod projection;
mod screenshot;
mod targets;
mod tile_pipeline;
mod ui_instances;
mod uniforms;

use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::Instant;

use rustc_hash::{FxHashMap, FxHashSet};

use glam::Mat4;
use wgpu::util::DeviceExt;

use mahjuro_core::core::relic::{RelicId, RelicRenderMaterial, relic_visual};
use mahjuro_core::core::tile::{Suit, Tile};
use mahjuro_core::core::tile_pack::TilePackKind;
use crate::abacus_mesh::{
    build_abacus_earth_beads_mesh, build_abacus_heaven_beads_mesh, build_abacus_mesh,
};
use crate::bell_tassel_mesh::build_bell_tassel_mesh;
use crate::bone_tablet_mesh::build_bone_tablet_mesh;
use crate::book_mesh::{build_book_body_mesh, build_book_cover_mesh};
use crate::cabinet_mesh::{build_cabinet_mesh, build_cabinet_rails_mesh};
use crate::coin_mesh::build_coin_mesh;
use crate::decal::{
    LabelAlign, load_mono_font, load_noto_emoji_font, load_ui_font,
    rasterize_label_styled_with_fallback, rasterize_tile_face_decal,
};
use crate::draw_cmd::{
    DrawCmd, ShowcaseTilePlacement, TileFaceQuad, UiFrame, WallStackPlacement, YakuTabletPlacement,
};
use crate::gpu_types::{DecodedRelicImage, RelicTextureGpu};
use crate::lit_mesh::Aabb;
use crate::lit_mesh::MeshCpu;
use crate::lit_mesh::push_box;
use crate::lit_mesh::{
    LitMeshGpu, LitMeshInstance, MaterialKind, MaterialParams, ShadowCasterUniform, ShadowGlobals,
    SsrGlobals, create_lit_mesh_material_layout, create_lit_mesh_spot_ssr_layout,
    create_room_env_camera_uniform_buffers, create_room_env_shadow_gpu_batch,
    create_shadow_caster_layout, create_shadow_sample_layout,
    create_shadow_warp_bind_group, create_shadow_warp_layout,
    ShadowDepthArrayGpu,
};
use crate::mirror_mesh::build_mirror_mesh;
use crate::ofuda_mesh::build_ofuda_mesh;
use crate::orb_mesh::build_orb_mesh;
use crate::plaque_mesh::build_plaque_mesh;
use crate::primitive::MeshId;
use crate::progress_meter_mesh::{
    build_progress_meter_pip_mesh, build_progress_meter_rail_mesh,
};
use crate::relic_dish::{
    build_dish_mesh, build_pack_mesh, build_porcelain_dish_mesh, build_relic_mesh,
    build_round_dish_mesh, build_shop_action_prop_mesh,
};
use crate::ribbon_mesh::build_ribbon_mesh;
use crate::river_mesh::build_river_mesh;
use crate::shop_bell_mesh::build_shop_bell_mesh;
use crate::table_transform::{
    ribbon_submesh, rot_fixed_axes_deg_matrix, tile_mesh_local_to_world, translate_rot_scale,
};
use crate::talisman_mesh::{TALISMAN_LOCAL_HALF, build_talisman_mesh};
use crate::tally_stick_mesh::{build_tally_stick_base_mesh, build_tally_stick_tip_mesh};
use crate::tile_glb::Vertex3dTex;
use crate::wood_tablet_mesh::build_wood_tablet_mesh;
use crate::world_space::pixel_to_world;
use mahjuro_types::scene_draw::BackgroundId;

use self::frame_pool::FrameBufferPool;
use self::resources::*;
use self::showcase::*;

pub struct WgpuRenderer {
    target: RenderTarget,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    /// Previous-frame depth snapshot copied after Pass A for lacquered-table SSR
    /// (paired with `scene_prev_texture`). See the SSR snapshot block in
    /// `runtime/render.rs`.
    ssr_prev_depth_texture: wgpu::Texture,
    ssr_prev_depth_view: wgpu::TextureView,
    /// Current-frame depth copied to R32Float before emissive-probe passes
    /// (`textureLoad` is unsupported on depth textures in the GLSL backend).
    depth_r32_snapshot_texture: wgpu::Texture,
    depth_r32_snapshot_view: wgpu::TextureView,
    depth_copy_staging_buffer: wgpu::Buffer,
    quad_pipeline: wgpu::RenderPipeline,
    /// Same shader as `quad_pipeline`, targeting the swapchain format (post-tonemap UI).
    quad_pipeline_display: wgpu::RenderPipeline,
    /// Quad pipeline variant that reads per-instance depth from `GpuInstance.user`.
    depth_quad_pipeline: wgpu::RenderPipeline,
    /// Display-format twin of `depth_quad_pipeline`.
    depth_quad_pipeline_display: wgpu::RenderPipeline,
    /// Rain debug: depth-colored variant of `depth_quad_pipeline`.
    depth_quad_debug_pipeline: wgpu::RenderPipeline,
    /// Display-format twin of `depth_quad_debug_pipeline`.
    depth_quad_debug_pipeline_display: wgpu::RenderPipeline,
    gradient_quad_pipeline: wgpu::RenderPipeline,
    squircle_quad_pipeline: wgpu::RenderPipeline,
    /// Display-format twin of `squircle_quad_pipeline` (post-tonemap UI).
    squircle_quad_pipeline_display: wgpu::RenderPipeline,
    /// Volumetric candle flame pipeline (`shaders/flame.wgsl` + `blackbody.wgsl`).
    flame_pipeline: wgpu::RenderPipeline,
    /// Shared revolved teardrop mesh; instanced once per candle.
    flame_volume_mesh: crate::lit_mesh::LitMeshGpu,
    /// Per-frame camera matrices uploaded for the flame vertex shader.
    /// Structurally identical to the view_proj + view_pos portion of
    /// `SsrGlobals`, but with a vertex-visible binding.
    flame_view_buffer: wgpu::Buffer,
    flame_view_bind_group: wgpu::BindGroup,
    /// Reusable staging buffer for [`crate::flame_volume::GpuFlameInstance`].
    flame_instance_staging: Vec<crate::flame_volume::GpuFlameInstance>,
    starfield_pipeline: wgpu::RenderPipeline,
    ember_drift_pipeline: wgpu::RenderPipeline,
    golden_dust_pipeline: wgpu::RenderPipeline,
    moonlit_water_pipeline: wgpu::RenderPipeline,
    // Owns the GPU resource that `moonlit_water_bind_group` samples from.
    moonlit_water_bind_group: wgpu::BindGroup,
    sunlit_water_pipeline: wgpu::RenderPipeline,
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
    /// Half-res downsample blit that publishes `scene_color_view` →
    /// `scene_prev_view` once per frame as the new SSR history input.
    /// Replaces the old full-res `copy_texture_to_texture` of color
    /// (~12 MB/frame at 1080p → ~3 MB/frame). Reuses
    /// `cascade_composite_layout` (texture + sampler) and the
    /// `cascade_composite_sampler`. See `scene_color_downsample.wgsl`.
    scene_color_downsample_pipeline: wgpu::RenderPipeline,
    scene_color_downsample_bind_group: wgpu::BindGroup,
    tile_pipeline_opaque_double: wgpu::RenderPipeline,
    tile_pipeline_opaque_cull: wgpu::RenderPipeline,
    tile_pipeline_blend_double: wgpu::RenderPipeline,
    tile_pipeline_blend_cull: wgpu::RenderPipeline,
    /// shop.glb only — glTF punctual + metallic-roughness + ACES (`room_glb.wgsl`).
    shop_pipeline_opaque_double: wgpu::RenderPipeline,
    shop_pipeline_opaque_cull: wgpu::RenderPipeline,
    shop_pipeline_blend_double: wgpu::RenderPipeline,
    shop_pipeline_blend_cull: wgpu::RenderPipeline,
    /// Shop emissive-only pre-pass (`room_glb` `fs_main_emissive` → `room_emissive_view`).
    shop_pipeline_mrt_opaque_double: wgpu::RenderPipeline,
    shop_pipeline_mrt_opaque_cull: wgpu::RenderPipeline,
    shop_pipeline_mrt_blend_double: wgpu::RenderPipeline,
    shop_pipeline_mrt_blend_cull: wgpu::RenderPipeline,
    /// Gold-metal shell behind outlined showcase tiles: merged mesh + instance
    /// vertex stream (model columns + rim factor), one draw per batch.
    tile_outline_pipeline: wgpu::RenderPipeline,
    /// Additive radial glow drawn behind selected tiles. A soft elliptical
    /// halo in warm gold that spills out past the tile silhouette and
    /// pulses gently with the candlelight rhythm.
    tile_glow_pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    tile_material_layout: wgpu::BindGroupLayout,
    /// Zeroed [`crate::hallway_glb::HallwayDistortion`] for `tile_3d` / showcase bind groups (binding 8).
    tile_env_distortion_placeholder: wgpu::Buffer,
    /// Group 0 for `tile_outline_pipeline`: frame uniform (instances are VB slot 1).
    tile_outline_frame_uniform_buffer: wgpu::Buffer,
    tile_outline_instance_buffer: wgpu::Buffer,
    tile_outline_frame_bind_group: wgpu::BindGroup,
    /// Built in `run_showcase_tiles_placement`, consumed by outline instanced draw.
    tile_outline_instances_staging: Vec<TileOutlineInstance>,
    /// Per `ShowcaseTileBatch` render op: `(first_instance, instance_count)` into staging / GPU buffer.
    tile_outline_batch_ranges: Vec<(u32, u32)>,
    /// Per-frame point-light array — gameplay / artist falloff (group 1 for tiles + lit_mesh).
    point_lights_buffer: wgpu::Buffer,
    tile_occluders_buffer: wgpu::Buffer,
    point_lights_bind_group: wgpu::BindGroup,
    /// Per-frame spotlight buffer (`SpotLightsBuf`). Tile pipeline and
    /// lit_mesh bind it as group 3.
    spot_lights_buffer: wgpu::Buffer,
    spot_lights_bind_group: wgpu::BindGroup,
    tile_sampler: wgpu::Sampler,
    /// Keeps default normal-map texture alive for [`TilePrimitiveGpu::normal_view`] clones.
    _tile_default_normal_texture: wgpu::Texture,
    _tile_glb_default_mr_texture: wgpu::Texture,
    _tile_glb_default_emissive_texture: wgpu::Texture,
    /// Per-material GPU tile meshes (`Bamboo`, `Plastic`, `TortoiseShell`).
    tile_meshes: [TileMeshGpuSet; crate::tile_glb::TILE_MATERIAL_MESH_COUNT],
    active_tile_material: mahjuro_gfx_types::TileMaterial,
    /// [`shop.glb`](../../assets/3d/shop.glb) environment primitives (tile vertex layout + materials).
    shop_env_primitives: Vec<TilePrimitiveGpu>,
    shop_environment: Option<ShopEnvironmentGpu>,
    /// glTF node TRS animation bindings for [`shop.glb`](../../assets/3d/shop.glb).
    shop_gltf_anim: crate::room_gltf_anim::RoomGltfAnimGpu,
    shop_gltf_anim_missing_clip_warned: std::cell::Cell<bool>,
    /// GPU primitive indices for the `Eyeball` node in [`Self::shop_env_primitives`].
    shop_eyeball_prim_indices: Vec<usize>,
    /// Bitset: which deferred room environments finished GPU upload (see `room_gpu_load.rs`).
    rooms_gpu_loaded: u8,
    /// Previous frame wall time (ms) for [`room_gpu_profile`] hitch logging.
    room_profile_frame_dt_ms: f32,
    shadow_warp_layout: wgpu::BindGroupLayout,
    tile_env_normal_view: wgpu::TextureView,
    tile_env_mr_view: wgpu::TextureView,
    tile_env_emissive_view: wgpu::TextureView,
    /// [`hallway.glb`](../../assets/3d/hallway.glb) pick-blind room.
    hallway_env_primitives: Vec<TilePrimitiveGpu>,
    hallway_environment: Option<ShopEnvironmentGpu>,
    /// [`staircase.glb`](../../assets/3d/staircase.glb) post-ordeal interstitial.
    staircase_env_primitives: Vec<TilePrimitiveGpu>,
    staircase_environment: Option<ShopEnvironmentGpu>,
    /// [`archive.glb`](../../assets/3d/archive.glb) Archive room.
    archive_env_primitives: Vec<TilePrimitiveGpu>,
    archive_environment: Option<ShopEnvironmentGpu>,
    /// [`main_menu.glb`](../../assets/3d/main_menu.glb) hub waterfront.
    main_menu_env_primitives: Vec<TilePrimitiveGpu>,
    main_menu_environment: Option<ShopEnvironmentGpu>,
    gameplay_env_primitives: Vec<TilePrimitiveGpu>,
    /// GPU primitive indices for `btn_cash_in` / `label_cash_in` (multi-material meshes).
    gameplay_cash_in_prim_indices: Vec<usize>,
    /// Primitive groups for each gameplay score roller slot (0..20).
    gameplay_score_roller_prim_groups: Vec<Vec<usize>>,
    /// Roller pivot points in gameplay.glb document space (parallel with slot index).
    gameplay_score_roller_pivots_doc: Vec<[f32; 3]>,
    /// Roller spin axes in gameplay.glb document space (parallel with slot index).
    gameplay_score_roller_axes_doc: Vec<[f32; 3]>,
    /// Smoothed mechanical drives for score / target banks.
    gameplay_score_roller_drive_values: std::cell::RefCell<[f64; 2]>,
    gameplay_score_roller_drive_initialized: std::cell::RefCell<[bool; 2]>,
    /// Continuous roll time while either bank is catching up (resets on stop, not retarget).
    gameplay_score_roller_roll_elapsed: std::cell::RefCell<f64>,
    /// Set when both odometer banks finish a spin; drained by the app after render.
    gameplay_score_roller_stopped: std::cell::RefCell<bool>,
    gameplay_environment: Option<ShopEnvironmentGpu>,
    /// GPU primitive index of `inspect_plaque` in `archive_env_primitives` (inspect decal host).
    archive_inspect_plaque_prim_idx: Option<usize>,
    /// GPU primitive index of `sign_description_left` in `archive_env_primitives` (for culling).
    archive_sign_left_prim_idx: Option<usize>,
    archive_sign_right_prim_idx: Option<usize>,
    /// All GPU primitive indices for `btn_page_left` / `btn_page_right` (multi-material meshes).
    archive_page_left_prim_indices: Vec<usize>,
    archive_page_right_prim_indices: Vec<usize>,
    /// Archive UI chrome that must not cast into the punctual shadow depth pass.
    archive_punctual_shadow_skip_prims: rustc_hash::FxHashSet<usize>,
    /// Last-uploaded browse-board decal; `u64::MAX` = cleared / none.
    archive_sign_decal_upload_key: u64,
    /// Last-uploaded inspect-plaque decal; `u64::MAX` = cleared / none.
    archive_inspect_plaque_decal_upload_key: u64,
    /// Per-scene room GLB tuning (linear/ambient/emissive/height). Filled each frame
    /// from [`crate::tuning::scene_look::SceneLookTuningSet`]; each GLB env pass
    /// reads its own scene key (shop, pick_chamber, gameplay, …).
    frame_env_tunes: rustc_hash::FxHashMap<&'static str, crate::room_glb::RoomEnvFrameTune>,
    /// Active scene's room values (tiles, bloom, lit_mesh when not env-specific).
    active_frame_env: crate::room_glb::RoomEnvFrameTune,
    /// CPU triangle soups from invisible marker meshes in [`shop.glb`](../../assets/3d/shop.glb).
    pub(super) shop_env_collision_meshes: Vec<crate::room_glb::RoomCollisionMesh>,
    /// Roof shells in [`main_menu.glb`](../../assets/3d/main_menu.glb) for doorway punctual occlusion.
    pub(super) main_menu_env_collision_meshes: Vec<crate::room_glb::RoomCollisionMesh>,
    /// CPU triangle soups from authored gameplay env buttons (`btn_cash_in`, …).
    pub(super) gameplay_env_collision_meshes: Vec<crate::room_glb::RoomCollisionMesh>,
    /// Identity factor used by every primitive (kept for the cam uniform).
    tile_base_color_factor: [f32; 4],
    /// Active tileset directory name (e.g. `"original"`). When `Some`, tile
    /// decals are loaded from `assets/textures/tile_sets/<name>/` instead of rasterized.
    tile_set: Option<String>,
    /// Per-hand-tile GPU resources; kept in sync with the hand via `update_hand_tiles`.
    hand_tiles: Vec<HandTileGpu>,
    /// Per-showcase-tile GPU resources (pack celebration, etc.). Grown on
    /// demand up to `MAX_SHOWCASE_TILE_SLOTS`; decals re-rasterised only
    /// when the tile identity changes.
    showcase_tiles: Vec<ShowcaseTileGpu>,
    /// Cached 2D tile-face overlays keyed by tile identity.
    tile_face_overlays:
        FxHashMap<(Suit, u8, Option<mahjuro_core::core::tile::TileEnhancement>, bool), TileFaceOverlayGpu>,
    /// Cached [`ImageQuadSource`] textures keyed by [`ImageQuadSource::cache_key`].
    image_quad_overlays: FxHashMap<String, TileFaceOverlayGpu>,
    /// Negative cache for [`Self::image_quad_overlays`]: keys whose upload
    /// already failed. Re-trying every frame would re-decode the sheet and
    /// flood the log; we warn once and skip thereafter.
    image_quad_missing: FxHashSet<String>,
    /// Lazily built texture + bind group for [`Object3dKind::Relic::debuffed`] overlays.
    debuff_marker_overlay: Option<TileFaceOverlayGpu>,
    /// Cached text-label rasterizations. Two-level map so the hit path can
    /// borrow `&str` for the text component (stdlib `HashMap<String, _>`
    /// accepts `&str` via `Borrow<str>`) — avoids cloning every label's text
    /// just to probe the cache. Rect/color animate per-frame and live on the
    /// per-frame instance buffer instead. Entries are evicted when their
    /// `last_used` frame stamp falls more than `TEXT_CACHE_TTL_FRAMES` behind
    /// `text_cache_frame`.
    text_label_cache: FxHashMap<TextLabelShapeKey, FxHashMap<String, CachedTextLabel>>,
    /// Monotonically increasing frame counter for `text_label_cache` eviction.
    text_cache_frame: u64,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    // --- Text overlay pipeline ---
    text_pipeline: wgpu::RenderPipeline,
    /// Same as `text_pipeline` but `Rgba16Float` color attachment (journal prepass).
    text_pipeline_scene_hdr: wgpu::RenderPipeline,
    text_bind_group_layout: wgpu::BindGroupLayout,
    /// Globals + text bind groups — shared by swapchain-format text/image pipelines.
    text_overlay_pipeline_layout: wgpu::PipelineLayout,
    text_shader_module: wgpu::ShaderModule,
    image_shader_module: wgpu::ShaderModule,
    // --- Image quad pipeline (full-colour textures for relic icons) ---
    image_pipeline: wgpu::RenderPipeline,
    image_pipeline_scene_hdr: wgpu::RenderPipeline,
    ui_font: Option<fontdue::Font>,
    emoji_font: Option<fontdue::Font>,
    ui_font_italic: Option<fontdue::Font>,
    mono_font: Option<fontdue::Font>,
    pub size: crate::physical_size::PhysicalSize,
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
    /// Previous frame: authored cash-in control drawn and pickable.
    pub(super) last_gameplay_cash_in_button_visible: bool,
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
    obj3d_hover_state: FxHashMap<u64, f32>,
    /// Creation time — used as a stable reference for cyclic animations.
    creation_time: Instant,
    /// Cached relic icon textures, populated asynchronously from the loader thread.
    relic_textures: FxHashMap<RelicId, RelicTextureGpu>,
    /// Per-frame relic draw slots — populated lazily via [`init_deferred`](init_deferred).
    relic_instances: Vec<LitMeshInstance>,
    gameplay_hud_pools_ready: bool,
    talisman_textures_ready: bool,
    /// Receives decoded relic RGBA data from the background loader thread.
    relic_rx: Option<mpsc::Receiver<DecodedRelicImage>>,
    /// Set when the background relic loader has finished (avoids respawning each frame).
    relic_load_finished: bool,
    /// Wall-clock start of the relic load pipeline (spawn → last GPU upload).
    relic_load_start: Option<Instant>,
    /// Cumulative main-thread mesh extraction + GPU mesh buffer creation during relic boot.
    /// Cumulative main-thread albedo + relief texture uploads during relic boot.
    relic_profile_upload_cpu: std::time::Duration,
    /// Cached tile-pack box art textures, keyed by `TilePackKind`.
    pack_textures: FxHashMap<TilePackKind, RelicTextureGpu>,
    /// Cached background textures, populated asynchronously.
    background_textures: FxHashMap<BackgroundId, BackgroundTextureGpu>,
    /// Receives decoded background image data from the background loader thread.
    background_rx: Option<mpsc::Receiver<DecodedBackgroundImage>>,
    /// Wall-clock start of the background load pipeline (spawn → last GPU upload).
    background_load_start: Option<Instant>,
    /// Per-hand-tile last-known world position keyed by tile uid (hand tiles
    /// with pick ids). Used for motion-aware effects and projection caching.
    prev_tile_world: FxHashMap<u32, glam::Vec3>,
    /// Reusable scratch set holding the live tile uids during the per-frame
    /// `prev_tile_world` GC. Cleared and re-populated each frame so the
    /// allocation amortizes across frames rather than churning a fresh
    /// `HashSet` every call.
    tile_uid_scratch: FxHashSet<u32>,
    /// Previous frame's shadow quality — forces shadow-map redraw when shadows enable.
    prev_shadow_quality: mahjuro_gfx_types::ShadowQuality,
    /// Last uploaded first projected-light view-proj — redraw when lights move.
    cached_shadow_light_view_proj: [f32; 16],
    /// Projected shadow depth setups for the current frame.
    projected_shadow_lights: Vec<crate::projected_light_shadow::ProjectedShadowLightSetup>,
    cached_projected_shadow_hash: u64,
    /// Lit-mesh slot for [`crate::scenes::shop::shared::SHOP_INSPECT_SUBJECT_ANIM_ID`] this frame.
    shop_inspect_subject_shadow_slot: Option<(runtime::DrawKind, usize)>,
    /// Current `Object3d::anim_id` while [`runtime::object3d_placement::WgpuRenderer::run_object3d_placement`] walks a batch.
    shadow_placement_anim_id: u64,
    /// Active imported room for per-frame Object3d shadow cast policy.
    placement_shadow_room:
        Option<crate::wgpu_renderer::runtime::shadow_setup::ActiveRoomEnv>,
    /// Whether the current Object3d placement iteration casts into the dynamic shadow map.
    placement_shadow_casts: bool,
    /// Pre-rasterized showcase tile decals + UV lookup (`showcase_decal_atlas.rs`).
    showcase_decal_atlas: Option<crate::showcase_decal_atlas::ShowcaseDecalAtlasGpu>,
    /// Tileset name the atlas was baked for; rebuilt when Options tileset changes.
    showcase_decal_atlas_tileset: Option<String>,
    /// In-memory cache of inactive showcase atlases keyed by tileset name.
    /// Splash preloading fills this so tileset cycling avoids rebuild hitches.
    showcase_decal_atlas_cache: VecDeque<(
        String,
        crate::showcase_decal_atlas::ShowcaseDecalAtlasGpu,
    )>,

    // ── Procedural lit meshes (candles + wood table) ────────────────────
    /// Bind-group layout shared by every lit-mesh instance.
    lit_mesh_material_layout: wgpu::BindGroupLayout,
    /// Bind-group layout for lit_mesh group 3: spotlights + SSR history.
    lit_mesh_spot_ssr_layout: wgpu::BindGroupLayout,
    /// Frame-shared SSR uniform (camera matrices + toggle + tuning).
    lit_mesh_ssr_buffer: wgpu::Buffer,
    /// Spotlights + SSR resources bound as lit_mesh group 3. Recreated on
    /// resize whenever the scene-history texture or SSR depth snapshot is reallocated.
    lit_mesh_spot_ssr_bind_group: wgpu::BindGroup,
    /// Sampler used by the SSR pass for both the scene-history colour
    /// texture and the depth snapshot.
    lit_mesh_ssr_sampler: wgpu::Sampler,
    /// Snapshot of the previous frame's linear HDR scene colour. Read by the
    /// lacquered floor as the SSR source.
    scene_prev_texture: wgpu::Texture,
    scene_prev_view: wgpu::TextureView,
    /// Current frame's linear HDR scene color (`Rgba16Float`), rendered offscreen
    /// before bloom, tonemap, and final composite into the swapchain.
    scene_color_texture: wgpu::Texture,
    scene_color_view: wgpu::TextureView,
    /// Emissive-only linear RGB (`room_glb` emissive pre-pass) for GI gather —
    /// excludes BRDF / punctual.
    room_emissive_texture: wgpu::Texture,
    room_emissive_view: wgpu::TextureView,
    /// Half-res emissive indirect estimate (linear HDR).
    emissive_gi_texture: wgpu::Texture,
    emissive_gi_view: wgpu::TextureView,
    emissive_probe_update_pipeline: wgpu::ComputePipeline,
    emissive_probe_update_bind_group_layout: wgpu::BindGroupLayout,
    emissive_probe_update_bind_group: wgpu::BindGroup,
    emissive_probe_apply_pipeline: wgpu::RenderPipeline,
    emissive_probe_apply_bind_group_layout: wgpu::BindGroupLayout,
    emissive_probe_apply_bind_group: wgpu::BindGroup,
    probe_gi_frame_uniform_buffer: wgpu::Buffer,
    probe_sh_buffer: wgpu::Buffer,
    /// GI frames since room GI became active (drives amortized probe compute).
    probe_gi_tick: u32,
    probe_gi_last_view_proj: [f32; 16],
    probe_gi_last_size: (u32, u32),
    probe_gi_had_room: bool,
    /// Which room's baked SH coefficients are currently in [`Self::probe_sh_buffer`].
    probe_gi_gpu_room: Option<crate::room_gi_bake::RoomGiRoom>,
    /// Set by `mahjuro bake-room-gi` to read back probes after one dynamic GI frame.
    room_gi_capture_pending: Option<crate::room_gi_bake::RoomGiRoom>,
    room_gi_capture_meta: Option<crate::room_gi_bake::RoomGiBake>,
    room_gi_captured: Option<crate::room_gi_bake::RoomGiBake>,
    /// Offline baked room shadow maps (one per static room); lazy-uploaded on first draw.
    room_baked_shadow_gpu: [Option<impl_room_shadow::RoomBakedShadowGpu>;
        crate::room_gi_bake::ROOM_GI_ROOM_COUNT],
    active_room_baked_shadow: Option<crate::room_gi_bake::RoomGiRoom>,
    room_shadow_capture_pending: Option<crate::room_gi_bake::RoomGiRoom>,
    room_shadow_captured: Option<crate::room_shadow_bake::RoomShadowBake>,
    shadow_probe_last_log: Instant,
    shadow_probe_last_caster_count: usize,
    emissive_gi_composite_pipeline: wgpu::RenderPipeline,
    emissive_gi_composite_bind_group_layout: wgpu::BindGroupLayout,
    emissive_gi_composite_bind_group: wgpu::BindGroup,
    /// Fullscreen offscreen target for the live GPU render of the
    /// yaku-journal scene. The book mesh's leather shader samples this
    /// in screen space (not UV) so the rendered scene reads as a window
    /// cut through the page region. Window-sized; resized in `resize()`.
    pub(crate) journal_scene_texture: wgpu::Texture,
    pub(crate) journal_scene_view: wgpu::TextureView,
    /// Generation counter, bumped every time `journal_scene_view` is
    /// recreated (resize, surface reconfigure, etc.). Cached bind groups
    /// that bind that view stamp this counter and re-bind when it
    /// drifts. Without it, a destroyed-texture validation error fires
    /// the first frame after resize because the cached bind group still
    /// holds a stale view.
    pub(crate) journal_scene_view_generation: u32,
    bloom_extract_pipeline: wgpu::RenderPipeline,
    bloom_blur_pipeline: wgpu::RenderPipeline,
    bloom_composite_pipeline: wgpu::RenderPipeline,
    bloom_bind_group_layout: wgpu::BindGroupLayout,
    bloom_extract_bind_group_layout: wgpu::BindGroupLayout,
    bloom_composite_bind_group_layout: wgpu::BindGroupLayout,
    /// Per-pass bloom uniforms (extract / blur axis / composite differ in `data1`).
    bloom_extract_params_buffer: wgpu::Buffer,
    bloom_blur_h_params_buffer: wgpu::Buffer,
    bloom_blur_v_params_buffer: wgpu::Buffer,
    bloom_composite_params_buffer: wgpu::Buffer,
    bloom_sampler: wgpu::Sampler,
    bloom_scene_bind_group: wgpu::BindGroup,
    bloom_ping_bind_group: wgpu::BindGroup,
    bloom_pong_bind_group: wgpu::BindGroup,
    bloom_composite_bind_group: wgpu::BindGroup,
    bloom_ping_texture: wgpu::Texture,
    bloom_ping_view: wgpu::TextureView,
    bloom_pong_texture: wgpu::Texture,
    bloom_pong_view: wgpu::TextureView,
    /// Scene + bloom composite in linear HDR before tonemap → swapchain / journal.
    post_bloom_texture: wgpu::Texture,
    post_bloom_view: wgpu::TextureView,
    tonemap_pipeline: wgpu::RenderPipeline,
    tonemap_rgba16f_pipeline: wgpu::RenderPipeline,
    tonemap_bind_group_layout: wgpu::BindGroupLayout,
    tonemap_params_buffer: wgpu::Buffer,
    tonemap_bind_group: wgpu::BindGroup,
    /// Alt tonemap bind group whose binding 1 points at `scene_color_view`
    /// (instead of `post_bloom_view`). Used to skip the scene-composite
    /// pass when bloom + fisheye + GI are all inactive. See `render.rs`.
    tonemap_bind_group_scene: wgpu::BindGroup,
    /// Per-frame bump-allocated GPU buffer pool. Reset at the top of
    /// every `render()` and used by `runtime/render.rs` for quad-batch,
    /// gradient/squircle quad, background and text-instance vertex
    /// uploads. See [`crate::wgpu_renderer::frame_pool`].
    frame_buffer_pool: FrameBufferPool,
    tonemap_shader_module: wgpu::ShaderModule,
    tonemap_pipeline_layout: wgpu::PipelineLayout,
    /// Surface format used when HDR is off (or unavailable).
    swapchain_sdr_format: wgpu::TextureFormat,
    /// Whether `Rgba16Float` was in the surface capabilities at init.
    swapchain_hdr_available: bool,
    /// Effective tonemap + VHS knobs for the next `render` call. Resolved
    /// per scene by the app (see `crate::game::tonemap_tuning`); pushed
    /// here once per frame and read in `render.rs` when assembling the
    /// `TonemapParams` upload. `tonemap_vhs_enabled` gates the VHS branch
    /// independently — when off, the per-amount values are still preserved
    /// so re-enabling restores the previous look without round-tripping.
    pub tonemap_exposure: f32,
    pub tonemap_vhs_enabled: bool,
    pub tonemap_vhs_chromatic: f32,
    pub tonemap_vhs_scanline: f32,
    pub tonemap_vhs_grain: f32,
    pub tonemap_vhs_vignette: f32,
    /// Increments each `render` call; re-rolls VHS tape grain without UV scroll.
    vhs_grain_frame: u32,
    /// Main-menu world rain tuning (CPU field). Live-tuned via Debug > Rain.
    pub main_menu_effects: crate::main_menu_effects_tuning::MainMenuEffectsTuning,
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
    /// Per-kind organic silhouette masks for shop talisman pendants — [`TalismanKind::all()`] order.
    talisman_mask_views: Vec<wgpu::TextureView>,
    /// Per-kind heightmaps for memorial (remnant) pendants — [`MemorialTalismanKind::all()`] order.
    memorial_talisman_height_views: Vec<wgpu::TextureView>,
    /// Per-kind organic silhouette masks for memorial pendants — [`MemorialTalismanKind::all()`] order.
    memorial_talisman_mask_views: Vec<wgpu::TextureView>,
    /// Per-kind mask-extruded pendant meshes (shop). Falls back to [`Self::talisman_mesh`].
    talisman_meshes: rustc_hash::FxHashMap<mahjuro_core::core::talisman::TalismanKind, LitMeshGpu>,
    /// Per-kind mask-extruded pendant meshes (memorial).
    memorial_talisman_meshes:
        rustc_hash::FxHashMap<mahjuro_core::core::memorial_talisman::MemorialTalismanKind, LitMeshGpu>,
    talisman_meshes_ready: bool,
    /// Which heightmap is currently bound per talisman slot. Used to skip
    /// rebinding when the kind hasn't changed since last frame. Indexed
    /// parallel with `talisman_instances`; `None` means the white fallback
    /// is still bound.
    talisman_slot_kind: Vec<Option<u8>>,
    /// Shared procedural meshes.
    relic_box_mesh: LitMeshGpu,
    /// Unit box for tile booster packs (correct UVs per face; avoids the relic
    /// cylinder's repeated side strips).
    pack_mesh: LitMeshGpu,
    /// Per-relic silhouette-derived meshes generated from the loaded relic
    /// texture alpha. Falls back to `relic_box_mesh` when no usable silhouette
    /// can be derived.
    relic_meshes: FxHashMap<RelicId, LitMeshGpu>,
    /// CPU-side triangle lists for the fallback relic box, used by the
    /// trimesh ray-picker when a relic's per-ID mesh isn't loaded yet.
    /// Built once at renderer init from `build_relic_mesh()`.
    pub(super) relic_box_tris: Vec<[glam::Vec3; 3]>,
    /// CPU-side triangle lists per relic, extracted from the same CPU mesh
    /// used to build `relic_meshes`. Drives per-triangle trimesh picking so
    /// the click silhouette matches the visible relic outline instead of a
    /// loose AABB slab.
    pub(super) relic_tri_lists: FxHashMap<RelicId, Vec<[glam::Vec3; 3]>>,
    /// Per-relic-placeholder (world-space model matrix, relic id) captured
    /// each frame for `pick_shop_object` raycasting and the bulk screen-rect
    /// reprojection. The relic id drives per-triangle trimesh picking so the
    /// click silhouette matches the visible relic outline.
    pub(super) last_relic_models: Vec<(Mat4, RelicId)>,
    /// Currently bound relic texture per slot. `Some(id)` means that slot's
    /// bind group already points at the texture for `id`; `None` means the
    /// flat-white fallback. Avoids rebuilding bind groups every frame.
    relic_slot_texture: Vec<Option<RelicId>>,
    /// Per-slot boss-icon meshes/textures (archive catalog).
    ordeal_icon_instances: Vec<LitMeshInstance>,
    ordeal_icon_meshes: rustc_hash::FxHashMap<mahjuro_core::core::ordeal_kind::OrdealKind, LitMeshGpu>,
    ordeal_icon_textures: rustc_hash::FxHashMap<mahjuro_core::core::ordeal_kind::OrdealKind, RelicTextureGpu>,
    ordeal_icon_slot_texture: Vec<Option<mahjuro_core::core::ordeal_kind::OrdealKind>>,
    /// Pre-allocated per-pack instances (lit-mesh foil; uses `pack_mesh` geometry).
    pack_instances: Vec<LitMeshInstance>,
    pack_slot_texture: Vec<Option<TilePackKind>>,
    // ── Shop scene meshes (curio cabinet + ribbons + talismans) ─
    ribbon_mesh: LitMeshGpu,
    talisman_mesh: LitMeshGpu,
    /// Per-ribbon draw-slot instances (shop scene). One slot per ribbon —
    /// the whole ribbon is a single textured mesh now. Truncated at
    /// `MAX_RIBBON_SLOTS`.
    ribbon_instances: Vec<LitMeshInstance>,
    /// Currently bound zodiac texture per ribbon slot. `Some(zodiac_idx)`
    /// maps to `ribbon_zodiac_tex.views[idx]`; `None` means the flat-white
    /// fallback is bound. Used to skip redundant bind-group rebuilds.
    ribbon_slot_zodiac: Vec<Option<u8>>,
    /// Per-zodiac silk ribbon textures (one tall portrait per zodiac).
    ribbon_zodiac_tex: ZodiacRibbonTextures,
    /// Per-talisman instances (shop scene). Indexed sequentially by
    /// `TalismanBatch` placement order; truncated at `MAX_TALISMAN_SLOTS`.
    talisman_instances: Vec<LitMeshInstance>,
    /// Chitin ellipsoid body mesh for main-menu door-light moths.
    bug_body_mesh: LitMeshGpu,
    /// Flat wing-pair mesh for main-menu door-light moths.
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
    /// Per-ribbon world-space model matrices for `pick_shop_object`.
    pub(super) last_ribbon_models: Vec<Mat4>,
    /// Per-batch ribbon slot counts: `last_ribbon_batch_slot_counts[batch_idx]`
    /// is how many draw-slots that batch consumed (3 per textured ribbon,
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
    book_mesh: LitMeshGpu,
    book_cover_mesh: LitMeshGpu,
    bowl_mesh: LitMeshGpu,
    mirror_mesh: LitMeshGpu,
    tally_stick_base_mesh: LitMeshGpu,
    tally_stick_tip_mesh: LitMeshGpu,
    yaku_tablet_instances: Vec<LitMeshInstance>,
    wood_tablet_instances: Vec<LitMeshInstance>,
    book_instances: Vec<LitMeshInstance>,
    /// Per-book front-cover instances. Drawn as a sibling to
    /// `book_instances` with an extra hinge rotation around the local
    /// spine axis so the cover can swing open. One slot per book.
    book_cover_instances: Vec<LitMeshInstance>,
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
    /// Per-extruded-glyph-popup instances. Each slot owns its own uniform
    /// buffer + bind group; the *mesh* it renders against is fetched from
    /// `extruded_glyph_meshes` per draw because each label has a different
    /// vertex/index buffer.
    extruded_glyph_instances: Vec<LitMeshInstance>,
    /// Lazy cache of GPU-uploaded glyph meshes keyed by label string. The
    /// CPU-side `GlyphMeshCache` (font + tessellation) lives next to it so
    /// the renderer can build a mesh on first sight of a new label string
    /// and reuse it on every subsequent frame the same string appears.
    glyph_cpu_cache: crate::glyph_mesh::GlyphMeshCache,
    extruded_glyph_meshes: FxHashMap<String, LitMeshGpu>,
    /// Shape registry for `Object3dKind::Primitive`. During the Phase-1
    /// migration, entries share GPU allocations with the legacy named
    /// fields (`plaque_mesh`, `cabinet_mesh`, …) via `Arc`. Once a
    /// legacy kind is deleted, the registry entry becomes the sole
    /// owner.
    primitive_meshes: FxHashMap<crate::primitive::MeshId, std::sync::Arc<LitMeshGpu>>,
    /// Per-shape instance pools for `Object3dKind::Primitive`. Keyed by
    /// `MeshId`; each `Vec` grows on-demand via `ensure_lit_mesh_pool`.
    primitive_instances: FxHashMap<crate::primitive::MeshId, Vec<LitMeshInstance>>,
    /// Per-shape texture overrides for primitive instances. When a
    /// shape has an entry here, `dispatch_primitive` binds the
    /// specified albedo + relief textures at instance creation instead
    /// of the default white + flat relief. Used by meshes whose
    /// material samples a heightmap (e.g. engraved coin faces).
    primitive_textures:
        FxHashMap<crate::primitive::MeshId, (wgpu::TextureView, wgpu::TextureView)>,
    /// Authored mesh + material slots from [`coin.glb`](../../../assets/3d/coin.glb).
    coin_glb_primitives: Vec<TilePrimitiveGpu>,
    /// Per-instance uniforms/bind groups for [`DrawKind::GltfCoin`].
    coin_glb_instances: Vec<GltfPropGpu>,
    /// Per-pick-id model matrix snapshot for primitive hit-testing.
    pub(super) last_primitive_pick_models: FxHashMap<u32, Mat4>,
    /// Three reusable lit-mesh instances for the debug world-axes overlay
    /// (one per axis: 0 = X red, 1 = Y green, 2 = Z blue). Drawn through
    /// the shared `relic_box_mesh` unit cube; per-frame uniforms position
    /// and stretch each instance into a thin colored bar.
    debug_axes_instances: Vec<LitMeshInstance>,
    /// Per-yaku-tablet world-space model matrices for `pick_gameplay_object`.
    /// Parallel with `last_projected_yaku_tablet_rects`.
    pub(super) last_yaku_tablet_models: Vec<Mat4>,
    /// Per-wood-tablet world-space model matrices for `pick_gameplay_object`.
    /// Index 0 = cash-in tablet when structure is committed.
    pub(super) last_wood_tablet_models: Vec<Mat4>,
    /// Discard bowl world-space model matrix for `pick_gameplay_object`.
    pub(super) last_bowl_model: Option<Mat4>,
    /// Bronze mirror world-space model matrix for `pick_gameplay_object`.
    pub(super) last_mirror_model: Option<Mat4>,
    /// Canonical scene-path prefix for the currently active scene — e.g.
    /// `"shop"` or `"gameplay"`. Set per-frame by `App` so the renderer can
    /// disambiguate shared mesh pipelines (e.g. `Object3dKind::Ofuda` is used
    /// by both shop and gameplay).
    active_scene_key: Option<&'static str>,

    // ── Shadow mapping ─────────────────────────────────────────────────
    /// Per-point-light projected depth layers sampled by scene shaders.
    point_shadow_array: ShadowDepthArrayGpu,
    /// Per-spot-light projected depth layers sampled by scene shaders.
    spot_shadow_array: ShadowDepthArrayGpu,
    shadow_sample_layout: wgpu::BindGroupLayout,
    shadow_compare_sampler: wgpu::Sampler,
    shadow_ao_sampler: wgpu::Sampler,
    _shadow_ao_white_texture: wgpu::Texture,
    shadow_ao_white_view: wgpu::TextureView,
    /// Bind-group layout for per-caster uniforms (group 0 of the shadow
    /// pipeline). Each `LitMeshInstance` and `HandTileGpu` owns one bind
    /// group built against this layout.
    shadow_caster_layout: wgpu::BindGroupLayout,
    /// Group 1 of the shadow VS — `HallwayDistortion` (zeroed ⇒ no warp). Bound for every shadow draw.
    shadow_warp_disabled_bind_group: wgpu::BindGroup,
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
    /// Room GLB shadow pass — double-sided so thin shelves / panels cast.
    shadow_pipeline_room_env: wgpu::RenderPipeline,
    /// Optional GPU timestamp profiler. Built once at startup; activated
    /// on demand from the Debug menu via `start_gpu_profile`.
    gpu_profiler: crate::gpu_profiler::GpuProfiler,
    /// When `Some`, the next `draw()` call copies the surface texture into
    /// a staging buffer right before `present()` and writes it as a PNG to
    /// this path. Cleared after the capture. Set via
    /// [`WgpuRenderer::queue_screenshot`].
    pending_screenshot: std::cell::Cell<Option<std::path::PathBuf>>,
    /// Per-frame swapchain acquire telemetry. Records how long each frame
    /// spends inside `get_current_texture` plus the outcome distribution
    /// (Outdated, Timeout, etc.) and self-emits a one-shot info summary
    /// after warmup. See [`runtime::AcquireTelemetry`].
    acquire_telemetry: runtime::AcquireTelemetry,
    /// Live shadow quality tier (mirrors per-frame render settings).
    shadow_quality: mahjuro_gfx_types::ShadowQuality,
    /// Live candle flame tuning (shader + placement); synced from scene / debug overlay.
    pub flame_tuning: crate::flame_tuning::FlameTuning,
    /// Main-menu moon tab: pride rainbow on moon / stars (defaults on in June).
    pub main_menu_pride_rainbow_debug: bool,
    /// Emissive overlay mesh for main-menu `rain_hit_*` shells (rain debug menu).
    pub(super) main_menu_rain_hit_debug_mesh: Option<LitMeshGpu>,
    pub(super) main_menu_rain_hit_debug_instance: Option<LitMeshInstance>,
    /// Log once per room when probe GI AABB drifts stale (see `render.rs`).
    probe_gi_stale_aabb_warned_room: Option<crate::room_gi_bake::RoomGiRoom>,
}

mod impl_loaders;
mod impl_pipelines;
mod impl_public;
mod impl_room_gi;
mod impl_room_shadow;
mod impl_screenshot;

pub use constants::{
    MAIN_MENU_PICK_MOON, MAIN_MENU_PICK_OPTIONS, MAIN_MENU_PICK_PLAY, MAIN_MENU_PICK_QUIT,
    MAX_BOOK_SLOTS,
    MAX_BOWL_SLOTS, MAX_BUG_SLOTS, MAX_COIN_GLTF_SLOTS, MAX_EXTRUDED_GLYPH_SLOTS,
    MAX_MIRROR_SLOTS, MAX_ORB_SLOTS, MAX_ORDEAL_ICON_SLOTS, MAX_POINT_LIGHTS, MAX_RELIC_SLOTS,
    MAX_RIBBON_SLOTS, MAX_SPOT_LIGHTS, MAX_TALISMAN_SLOTS, MAX_TALLY_FAN_SLOTS,
    MAX_TALLY_STICK_SLOTS, MAX_TILE_OCCLUDERS, MAX_WALL_TILE_SLOTS, MAX_WOOD_TABLET_SLOTS,
    MAX_YAKU_TABLET_SLOTS, MEMORIAL_TALISMAN_TEXTURE_BASE,
};
pub use internal_slots::{RelicIcon, TextAlign, TextLabel};
pub use layout_instances::build_instances_from_layout;
pub use picking_types::{GameplayPick, MainMenuPick, ShopHit};
pub use projection::ProjectionCache;
pub use targets::TargetInit;
pub use ui_instances::{GpuInstance, GradientQuadInstance, RenderSettings};

pub(crate) use constants::clamp_render_physical_size;
pub(crate) use hash_util::tablet_label_hash;
pub use lighting_buffers::{PointLight, SpotLight};
pub(crate) use runtime::shadow_setup::ActiveRoomEnv;

pub(crate) use lighting_buffers::{
    PointLightsBuf, PunctualLightBakeParams, PunctualLightBakeShopCameraParams, SpotLightsBuf,
    TileOccluderGpu, TileOccludersBuf,
};
pub(crate) use moon::current_moon_phase;
pub(crate) use resources::{create_depth, create_depth_r32_snapshot};
pub(crate) use screenshot::ScreenshotStaging;
pub(crate) use targets::RenderTarget;
pub(crate) use tile_pipeline::{TileGlbPipelineKey, TileMeshGpuSet, TilePrimitiveGpu};
pub(crate) use uniforms::{
    BloomParams, CameraUniform, FlameViewUniform, Globals, ProbeGiFrameUniform,
    TileOutlineFrameUniform, TileOutlineInstance, TonemapParams,
};

pub(super) use constants::{
    LOCAL_X_EXTENT, LOCAL_Y_EXTENT, LOCAL_Z_EXTENT, MAX_SHOWCASE_TILE_SLOTS, TEXT_CACHE_TTL_FRAMES,
};
pub(crate) use projection::PickCamera;

pub(crate) use internal_slots::{
    CachedTextLabel, GltfPropGpu, HandTileGpu, ShopEnvironmentGpu, ShowcaseTileGpu, TextLabelShapeKey,
    TileFaceOverlayGpu,
};
