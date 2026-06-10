//! Generic room GLB infrastructure ([`RoomGlbCpu`], [`RoomEnvLightingTune`], shared camera/transform helpers)
//! shared by shop, hallway, and archive scenes; also owns the shop-specific static loader for
//! [`shop.glb`](../../../assets/3d/shop.glb).
//!
//! Marker object names (Blender object names → glTF node names):
//! - `exit_btn`, `restock_btn`, `journal_btn`
//! - [`PLAYER_GOLD_DISH_MARKER`] (legacy: `PlayerGoldDish`) — origin for the procedural gold coin pile (place at the dish floor).
//! - `shop_spawn_relic_00` … `shop_spawn_relic_08`
//! - `shop_player_relic_00` … `shop_player_relic_04`
//! - `shop_player_consumable_00`, `shop_player_consumable_01`
//!
//! **Spawn / inventory anchor** nodes (`shop_spawn_relic_*`, `shop_player_*`) may carry mesh
//! geometry that exists only for authoring (invisible hit volumes). That mesh is **skipped** at
//! decode time so it does not draw, but it is still decoded into **[`RoomCollisionMesh`]** triangle
//! soups for cursor ray picking (`pick_shop_object`). **Shop buttons** (`exit_btn`, `restock_btn`,
//! `journal_btn`) still record marker transforms **and** decode their meshes for drawing.
//!
//! ## Materials
//! Each primitive uses glTF PBR **base color texture** (if present) and multiplies by
//! **`baseColorFactor`** on the CPU. Factor-only materials become a 1×1 uploaded texture.
//! **Normal maps** (`material.normalTexture`) are decoded as linear RGBA; **`scale`** is baked
//! into texels. Tangents come from the glTF `TANGENT` attribute when present, otherwise from
//! [`crate::tile_glb::compute_vertex_tangents`] using the normal map TEXCOORD when it
//! differs from base color. Metallic–roughness, emissive, alpha modes, `COLOR_0`, and glTF sampler
//! settings follow [`crate::tile_glb::LoadedPrimitive`] (shared with `tile.glb`).
//!
//! ## Export (Blender / glTF)
//! Ship **`shop.glb` without Draco** (`KHR_draco_mesh_compression`). This crate uses
//! [`gltf::import_slice`](https://docs.rs/gltf), which does not decode Draco — compressed files fail
//! validation (`accessor.bufferView: Missing data`, unsupported extension).
//!
//! **Blender parity:** In the glTF 2.0 exporter use **Lighting Mode → Standard** (Khronos cd / lx
//! units). See the [Blender glTF manual](https://www.blender.org/manual/en/latest/addons/import_export/scene_gltf2.html)
//! (*Data → Lighting*). Re-export after changing light units so authored intensities match
//! [`KHR_lights_punctual`](https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_lights_punctual/README.md).
//!
//! **Validation:** Compare punctual brightness and cones in a reference viewer (e.g.
//! [Don McCurdy’s glTF Viewer](https://gltf-viewer.donmccurdy.com/)) against this build.
//!
//! **Blender parity:** The viewport almost always adds **World** lighting (sky/ground) and often
//! EEVEE indirect; glTF export does not include World. Runtime approximates that with hemispheric
//! ambient in `room_glb.wgsl` ([`SHOP_ENV_AMBIENT_SCALE`] / [`RoomEnvLightingTune`]) plus emissive-probe GI from
//! candles/lanterns. In Blender before export: glTF *Data → Lighting* units match Khronos, disable
//! viewport-only overlays, and compare in Don McCurdy’s viewer at exposure **−9** (same as
//! [`ROOM_GLB_LINEAR_EXPOSURE_BASE`]). Warm candle/lantern tints are intentional game grading.
//!
//! ## Scale & framing
//! Environment vertices are **centered** using the axis-aligned bounds of all shop mesh geometry:
//! the GPU model is `translate(-center * s) * uniformScale(s)` with `s = window_h * height_scale`, so
//! the room’s geometric center sits at world origin and stays on-screen as resolution changes.
//! The perspective camera (embedded or fallback) is offset the same way; vertical FOV is **raised**
//! only when needed so the bounds’ corners stay inside the frustum at the current aspect ratio.
//! Default multiplier is [`SHOP_ENV_HEIGHT_SCALE`]; Debug → Tuning → **Shop Env & Lighting…**
//! overrides height scale and [`RoomEnvLightingTune`] fields live (typical height range `0.001`–`2.0`).
//!
//! ## Optional perspective camera
//! If the default scene contains a **perspective** camera node, the shop uses it for
//! [`crate::draw_cmd::CameraParams`] (eye / target / up / vertical FOV). Transforms are read
//! in glTF camera convention (−Z forward, +Y up); positions are scaled by [`room_env_world_scale`]
//! like marker geometry. If multiple cameras exist, a node named `ShopCamera`, `shop_camera`, or
//! `Camera` wins; otherwise the first perspective camera in depth-first order is used. Orthographic
//! cameras are ignored (hardcoded fallback camera applies).
//!
//! ## `KHR_lights_punctual`
//! **Point** and **spot** lights on scene nodes drive shop lighting when present: hardcoded lamp +
//! fill point lights are omitted so only glTF punctual lights apply (hover highlights may still add
//! extras). **Directional** lights are skipped. With embedded lights, the room draws through
//! `room_glb.wgsl`: inverse-square attenuation (Khronos range window),
//! metallic–roughness, ACES (fitted) tonemap, and linear HDR exposure:
//! [`ROOM_GLB_LINEAR_EXPOSURE_BASE`] × debug tune (see [`SHOP_ENV_LINEAR_EXPOSURE`]) before tonemap;
//! [`SHOP_ENV_AMBIENT_SCALE`] defaults to `0` for this interior.
//! glTF punctual intensity is scaled by [`SHOP_GLTF_LIGHT_INTENSITY_SCALE`] (default `1`). Shop punctual
//! points use a separate uniform buffer, bound as group 1 binding 0 for [`room_glb.wgsl`] and binding 2
//! for [`lit_mesh.wgsl`] (inverse-square on props; stays within WebGPU `max_bind_groups` on Metal).
//! Punctual lights on nodes whose names start with [`SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX`] or
//! [`SHOP_GLTF_LANTERN_LIGHT_NODE_PREFIX`] multiply glTF color by
//! [`RoomEnvLightingTune::candle_light_color_mul`] / [`RoomEnvLightingTune::lantern_light_color_mul`];
//! other lights keep glTF-authored color.
//! `range` maps to glTF max distance (`0` = infinite).
//!
//! **Tonemap pipeline:** the room-env fragment shader (`room_glb.wgsl`) writes **linear HDR**
//! into `scene_color` (`Rgba16Float`). The single ACES (fitted) tonemap is applied by
//! `tonemap_composite.wgsl` after bloom composite. Room passes now use explicit
//! `RoomEnvUniform` fields (including `room_post_params`) instead of tile-path
//! `hdr_tonemap` overloading.
//!
//! Shared glTF room decode (meshes, lights, cameras, collision) lives in [`crate::room_env_gltf`].

use parking_lot::RwLock;

use rustc_hash::FxHashMap;

use crate::draw_cmd::CameraParams;
use crate::room_env_gltf::{
    self as renv, EmbeddedCameraHarvest, RoomEnvWalkHooks, RoomEnvWalkState, RoomMeshPolicy,
    marker_translation_doc, room_env_model_matrix_from_bounds_doc, walk_room_env_node,
};
use crate::tile_glb::release_loaded_primitive_gpu_source_buffers;
use anyhow::Context;
use glam::{Mat4, Vec3};

enum RoomGlbCache {
    Uninit,
    Ready(Option<Box<RoomGlbCpu>>),
}

static ROOM_GLB_CPU: RwLock<RoomGlbCache> = RwLock::new(RoomGlbCache::Uninit);

/// True when `shop.glb` has been decoded into the process cache (meshes may already be
/// released after a prior GPU upload).
pub fn shop_cpu_decoded() -> bool {
    let g = ROOM_GLB_CPU.read();
    matches!(&*g, RoomGlbCache::Ready(Some(_)))
}

/// True when decoded environment meshes are present and not yet released for GPU upload.
pub fn shop_cpu_ready_for_gpu_upload() -> bool {
    let g = ROOM_GLB_CPU.read();
    match &*g {
        RoomGlbCache::Ready(Some(cpu)) => {
            !cpu.environment_primitives.is_empty() && !cpu.environment_primitives_released
        }
        _ => false,
    }
}

/// Decode `shop.glb` into the process-wide CPU cache (main or prefetch thread).
pub fn decode_shop_glb_into_cache() {
    let mut w = ROOM_GLB_CPU.write();
    if matches!(&*w, RoomGlbCache::Ready(Some(cpu)) if !room_glb_cpu_needs_environment_mesh_reload(cpu)
        && !room_glb_cpu_stale_environment_for_gpu_upload(cpu))
    {
        return;
    }
    let ready = if let Some(file) = mahjuro_assets::asset_path::get("3d/shop.glb") {
        match load_shop_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::trace!(
                    "shop.glb: {} marker node(s), {} draw primitive(s), {} collision mesh(es)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                    cpu.collision_meshes.len(),
                );
                log::info!(
                    "shop.glb: node bind poses={} glTF anim clips={}",
                    cpu.node_bind_poses.len(),
                    cpu.gltf_anim_library.clips.len()
                );
                if cpu.embedded_perspective_camera.is_some()
                    || !cpu.embedded_point_lights.is_empty()
                    || !cpu.embedded_spot_lights.is_empty()
                {
                    log::trace!(
                        "shop.glb scene extras: perspective_camera={} point_lights={} spot_lights={}",
                        cpu.embedded_perspective_camera.is_some(),
                        cpu.embedded_point_lights.len(),
                        cpu.embedded_spot_lights.len(),
                    );
                }
                Some(cpu)
            }
            Err(e) => {
                panic!("shop.glb failed to load: {e:#}");
            }
        }
    } else {
        panic!("shop.glb not embedded; required when loading shop room");
    };
    *w = RoomGlbCache::Ready(ready.map(Box::new));
}

fn ensure_shop_glb_loaded() {
    crate::room_preload::join_shop_cpu_prefetch_blocking();
    let mut w = ROOM_GLB_CPU.write();
    match &*w {
        RoomGlbCache::Uninit => {}
        RoomGlbCache::Ready(Some(cpu)) if room_glb_cpu_needs_environment_mesh_reload(cpu) => {
            *w = RoomGlbCache::Uninit;
        }
        _ => return,
    }
    drop(w);
    decode_shop_glb_into_cache();
}

/// Read-only access to decoded shop data (markers, lights, collision, …).  
/// Do not call [`release_shop_environment_cpu_sources_after_gpu_upload`] from inside `f` (deadlock).
pub fn with_shop_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_shop_glb_loaded();
    let g = ROOM_GLB_CPU.read();
    match &*g {
        RoomGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        RoomGlbCache::Ready(None) => f(None),
        RoomGlbCache::Uninit => unreachable!(),
    }
}

/// Drops environment mesh + decoded texture RAM after [`crate::wgpu_renderer::WgpuRenderer`]
/// has uploaded shop draws to the GPU. Safe to call once at init; no-op if shop failed to load.
pub fn release_room_environment_primitives_cpu(cpu: &mut RoomGlbCpu) {
    for env in &mut cpu.environment_primitives {
        release_loaded_primitive_gpu_source_buffers(&mut env.mesh);
    }
    cpu.environment_primitives.clear();
    cpu.environment_primitives_released = true;
}

/// Corrupt / partial decode — environment bounds exist but mesh buffers were never uploaded.
pub(crate) fn room_glb_cpu_needs_environment_mesh_reload(cpu: &RoomGlbCpu) -> bool {
    cpu.environment_bounds_doc.is_some()
        && cpu.environment_primitives.is_empty()
        && !cpu.environment_primitives_released
}

/// After [`release_room_environment_primitives_cpu`], environment meshes must be re-parsed
/// before another GPU upload. Metadata-only accessors ([`with_shop_glb_cpu`], picking, punctual
/// lights) must **not** trigger a reload — they only need bounds, markers, and collision soups.
pub(crate) fn room_glb_cpu_stale_environment_for_gpu_upload(cpu: &RoomGlbCpu) -> bool {
    cpu.environment_primitives_released
        && cpu.environment_primitives.is_empty()
        && cpu.environment_bounds_doc.is_some()
}

pub fn release_shop_environment_cpu_sources_after_gpu_upload() {
    let mut g = ROOM_GLB_CPU.write();
    if let RoomGlbCache::Ready(Some(cpu)) = &mut *g {
        release_room_environment_primitives_cpu(cpu);
    }
}

/// Drop the entire shop CPU cache (low-memory GPU eviction path).
pub fn clear_shop_glb_cpu_cache() {
    *ROOM_GLB_CPU.write() = RoomGlbCache::Uninit;
}

/// Default height multiplier for [`room_env_world_scale`] when no debug override is active.
pub const SHOP_ENV_HEIGHT_SCALE: f32 = 1.0;

/// Multiplies glTF punctual **intensity** before upload (document-space inverse-square; see
/// `decal_atlas_uv.y` / `SsrGlobals.shop_punctual.x`). `1.0` matches authored glTF / Blender
/// export; lower only if punctuals clip after ACES (debug **glTF light intensity** slider).
pub const SHOP_GLTF_LIGHT_INTENSITY_SCALE: f32 = 1.0;

/// Default shared linear HDR gain for embedded GLB rooms (shop, hallway, archive, main menu):
/// `2^-9` ≈ Don McCurdy glTF viewer exposure **−9** (EV on linear HDR). Overridable per scene via
/// [`RoomEnvLightingTune::linear_exposure_base`]; multiplied with
/// [`RoomEnvLightingTune::linear_exposure`] before tonemap in `room_glb.wgsl` / matching `lit_mesh` paths.
pub const ROOM_GLB_LINEAR_EXPOSURE_BASE: f32 = 1.0 / 512.0; // 2^-9

/// Extra multiplier on room glTF emissive (`RoomEnvUniform.room_env_params.z`), after
/// `KHR_materials_emissive_strength` and `emissiveFactor × emissiveTexture`. Keep near `1` when
/// assets use emissive strength; raise only if authors omit the extension.
pub const SHOP_GLTF_EMISSIVE_SCALE: f32 = 1.0;

/// Linear HDR strength for emissive probe indirect on shop / hallway GLB (additive before tonemap).
pub const SHOP_ROOM_EMISSIVE_GI_STRENGTH: f32 = 0.42;

/// 3D probe grid resolution (world AABB from room corners). Product must be ≤ [`ROOM_EMISSIVE_PROBE_MAX`].
pub const ROOM_EMISSIVE_PROBE_GRID: [u32; 3] = [7, 4, 6];

pub const ROOM_EMISSIVE_PROBE_MAX: u32 = 256;

pub const ROOM_EMISSIVE_PROBE_DIR_SAMPLES: u32 = 20;

pub const ROOM_EMISSIVE_PROBE_MARCH_STEPS: u32 = 14;

/// Max ray length in world units for probe → emissive screen-space march.
pub const ROOM_EMISSIVE_PROBE_MARCH_WORLD: f32 = 28.0;

/// Recompute volumetric probe SH every N GI frames unless the view or resolution changed.
pub const ROOM_EMISSIVE_PROBE_UPDATE_INTERVAL: u32 = 2;

/// Element-wise view_proj delta above which probes refresh immediately (camera nudge / cut).
pub const ROOM_EMISSIVE_PROBE_VIEW_EPS: f32 = 2e-4;

/// Amortized GI probe refresh state (tick, last view, last size).
pub struct ProbeGiUpdateState {
    pub tick: u32,
    pub last_view_proj: [f32; 16],
    pub last_size: (u32, u32),
    pub had_room: bool,
}

/// Frame inputs for [`probe_gi_should_update_probes`].
pub struct ProbeGiUpdateParams<'a> {
    pub view_proj: &'a [f32; 16],
    pub size: (u32, u32),
    pub gi_active: bool,
    pub update_interval: u32,
}

/// Whether to run `emissive-probe-update` this frame (amortized GI). Resets when GI is inactive.
pub fn probe_gi_should_update_probes(
    state: &mut ProbeGiUpdateState,
    params: &ProbeGiUpdateParams<'_>,
) -> bool {
    if !params.gi_active {
        state.tick = 0;
        state.had_room = false;
        return false;
    }

    let first_room_frame = !state.had_room;
    state.had_room = true;

    let view_moved = params
        .view_proj
        .iter()
        .zip(state.last_view_proj.iter())
        .any(|(a, b)| (*a - *b).abs() > ROOM_EMISSIVE_PROBE_VIEW_EPS);
    let resized = state.last_size != params.size;
    let interval = params.update_interval.max(1);
    let on_interval = state.tick.is_multiple_of(interval);
    let update = first_room_frame || view_moved || resized || on_interval;

    if update {
        state.last_view_proj = *params.view_proj;
        state.last_size = params.size;
    }
    state.tick = state.tick.wrapping_add(1);
    update
}

/// Tighten or expand the room AABB used for probe placement (`pad_frac` of the box size per axis).
pub fn room_probe_world_aabb(corners: &[Vec3], pad_frac: f32) -> Option<(Vec3, Vec3)> {
    if corners.is_empty() {
        return None;
    }
    let mut mn = corners[0];
    let mut mx = corners[0];
    for c in corners.iter().skip(1) {
        mn = mn.min(*c);
        mx = mx.max(*c);
    }
    let diag = mx - mn;
    let pad = diag * pad_frac + Vec3::splat(1e-3);
    Some((mn - pad, mx + pad))
}

/// Default **tuning** multiplier for linear HDR (debug overlay). With embedded glTF punctual
/// lights, multiplied by [`ROOM_GLB_LINEAR_EXPOSURE_BASE`] before tonemap.
pub const SHOP_ENV_LINEAR_EXPOSURE: f32 = 1.0;

/// Hemispheric fill in `room_glb.wgsl` (`decal_atlas_uv.x`). Authoring default is 0
/// (punctual-forward interior); tune via debug overlay [`RoomEnvLightingTune::ambient_scale`].
pub const SHOP_ENV_AMBIENT_SCALE: f32 = 0.0;

/// Applied to `lit_mesh` as punctual buffer `extras.w` when
/// [`crate::draw_cmd::SceneLighting::embedded_gltf_punctual`] is set (`room_glb.wgsl` ignores it).
pub const SHOP_LIT_MESH_GLTF_PUNCTUAL_SCALE: f32 = 2.0;

/// Inverse-square intensity scale for hand/structure tiles (`tile_3d.wgsl` via
/// `TileUniform.tile_punctual_params.x`). Tiles share the same candle pools as `lit_mesh`.
pub const TILE_GLTF_PUNCTUAL_SCALE: f32 = 1.0;

/// glTF **node** name prefix for punctual lights that should read as warm candles
/// (`light_candle`, `light_candle.001`, `light_candle_06`, …).
pub const SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX: &str = "light_candle";

/// Linear RGB multiplier for punctual lights on nodes matching [`SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX`].
/// Warm shift for candle reads; other lights keep glTF linear RGB.
pub const SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL: [f32; 3] =
    crate::theme::color::rgb(crate::theme::color::PARCHMENT);

/// glTF **node** name prefix for punctual lights that should read as lanterns
/// (`light_lantern`, `light_lantern.001`, `light_lantern_06`, …).
pub const SHOP_GLTF_LANTERN_LIGHT_NODE_PREFIX: &str = "light_lantern";

/// Linear RGB multiplier for punctual lights on nodes matching [`SHOP_GLTF_LANTERN_LIGHT_NODE_PREFIX`].
/// Warm shift for lantern reads; other lights keep glTF linear RGB.
pub const SHOP_GLTF_LANTERN_LIGHT_COLOR_MUL: [f32; 3] =
    crate::theme::color::rgb(crate::theme::color::GOLD);

/// Runtime shop lighting matching the `SHOP_*` source constants. Carried on [`DrawCtx`](crate::scenes::DrawCtx)
/// and editable from the debug overlay.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoomEnvLightingTune {
    pub gltf_light_intensity_scale: f32,
    pub linear_exposure: f32,
    /// Shared linear HDR base (`2^-9` default) before tonemap when embedded glTF punctual is on.
    #[serde(default = "linear_exposure_base_default")]
    pub linear_exposure_base: f32,
    pub ambient_scale: f32,
    pub lit_mesh_gltf_punctual_scale: f32,
    #[serde(default = "tile_gltf_punctual_scale_default")]
    pub tile_gltf_punctual_scale: f32,
    /// Room glTF emissive strength ([`SHOP_GLTF_EMISSIVE_SCALE`] default).
    pub gltf_emissive_scale: f32,
    pub candle_light_color_mul: [f32; 3],
    #[serde(default = "lantern_light_color_mul_default")]
    pub lantern_light_color_mul: [f32; 3],
}

fn tile_gltf_punctual_scale_default() -> f32 {
    TILE_GLTF_PUNCTUAL_SCALE
}

fn linear_exposure_base_default() -> f32 {
    ROOM_GLB_LINEAR_EXPOSURE_BASE
}

fn lantern_light_color_mul_default() -> [f32; 3] {
    SHOP_GLTF_LANTERN_LIGHT_COLOR_MUL
}

impl Default for RoomEnvLightingTune {
    fn default() -> Self {
        Self::SOURCE_DEFAULTS
    }
}

impl RoomEnvLightingTune {
    pub const SOURCE_DEFAULTS: Self = Self {
        gltf_light_intensity_scale: SHOP_GLTF_LIGHT_INTENSITY_SCALE,
        linear_exposure: SHOP_ENV_LINEAR_EXPOSURE,
        linear_exposure_base: ROOM_GLB_LINEAR_EXPOSURE_BASE,
        ambient_scale: SHOP_ENV_AMBIENT_SCALE,
        lit_mesh_gltf_punctual_scale: SHOP_LIT_MESH_GLTF_PUNCTUAL_SCALE,
        tile_gltf_punctual_scale: TILE_GLTF_PUNCTUAL_SCALE,
        gltf_emissive_scale: SHOP_GLTF_EMISSIVE_SCALE,
        candle_light_color_mul: SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL,
        lantern_light_color_mul: SHOP_GLTF_LANTERN_LIGHT_COLOR_MUL,
    };

    /// Linear HDR gain before tonemap when embedded glTF punctual is active.
    pub fn room_glb_linear_hdr_gain(&self) -> f32 {
        self.linear_exposure * self.linear_exposure_base
    }
}

/// GPU + collision scale for one scene's room GLB pass (brownout already applied on room fields).
#[derive(Clone, Copy, Debug)]
pub struct RoomEnvFrameTune {
    pub linear_exposure: f32,
    pub linear_exposure_base: f32,
    pub ambient_scale: f32,
    pub lit_mesh_gltf_punctual_scale: f32,
    pub tile_gltf_punctual_scale: f32,
    pub gltf_emissive_scale: f32,
    pub height_scale: f32,
}

impl RoomEnvFrameTune {
    pub fn from_room_and_height(room: RoomEnvLightingTune, height_scale: f32) -> Self {
        Self {
            linear_exposure: room.linear_exposure,
            linear_exposure_base: room.linear_exposure_base,
            ambient_scale: room.ambient_scale,
            lit_mesh_gltf_punctual_scale: room.lit_mesh_gltf_punctual_scale,
            tile_gltf_punctual_scale: room.tile_gltf_punctual_scale,
            gltf_emissive_scale: room.gltf_emissive_scale,
            height_scale,
        }
    }

    /// Linear HDR gain before tonemap when embedded glTF punctual is active.
    pub fn room_glb_linear_hdr_gain(&self) -> f32 {
        self.linear_exposure * self.linear_exposure_base
    }
}

impl Default for RoomEnvFrameTune {
    fn default() -> Self {
        Self::from_room_and_height(RoomEnvLightingTune::SOURCE_DEFAULTS, SHOP_ENV_HEIGHT_SCALE)
    }
}

// --- Stable `Shop*` names (shared decode in `room_env_gltf`) ---
pub type RoomEnvPrimitiveCpu = renv::RoomEnvPrimitiveCpu;
pub type RoomCollisionMesh = renv::RoomCollisionMesh;
pub type RoomEnvironmentBounds = renv::RoomEnvironmentBounds;
pub type RoomGlbEmbeddedPointLight = renv::RoomGltfEmbeddedPointLight;
pub type RoomGlbEmbeddedSpotLight = renv::RoomGltfEmbeddedSpotLight;
pub type RoomGlbEmbeddedCamera = renv::RoomGltfEmbeddedCamera;

pub use crate::room_env_gltf::glb_punctual_range_world_upload;
pub(crate) use crate::room_env_gltf::room_environment_bounds;

#[inline]
pub fn room_env_world_scale(window_h: f32, height_scale: f32) -> f32 {
    renv::room_env_world_scale(window_h, height_scale)
}

#[inline]
pub fn room_env_model_matrix_from_cpu(window_h: f32, height_scale: f32, cpu: &RoomGlbCpu) -> Mat4 {
    room_env_model_matrix_from_bounds_doc(window_h, height_scale, cpu.environment_bounds_doc)
}

pub fn room_world_bounds_corners_centered(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
) -> Vec<Vec3> {
    let Some(bounds) = cpu.environment_bounds_doc else {
        return Vec::new();
    };
    renv::room_world_bounds_corners_centered(bounds, window_h, env_height_scale)
}

pub fn room_node_mesh_center_world(
    cpu: &RoomGlbCpu,
    window_h: f32,
    height_scale: f32,
    node_name: &str,
) -> Option<Vec3> {
    let room_center_doc = cpu
        .environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(Vec3::ZERO);
    let s = room_env_world_scale(window_h, height_scale);
    let mut mn = Vec3::splat(f32::INFINITY);
    let mut mx = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for ep in &cpu.environment_primitives {
        if ep.gltf_node_name.as_deref() != Some(node_name) {
            continue;
        }
        for v in &ep.mesh.vertices {
            let p = Vec3::from_array(v.position);
            mn = mn.min(p);
            mx = mx.max(p);
            any = true;
        }
    }
    if any {
        let center_doc = (mn + mx) * 0.5;
        return Some((center_doc - room_center_doc) * s);
    }
    cpu.node_bind_poses
        .get(node_name)
        .map(|bind| (bind.bind_world_doc.transform_point3(Vec3::ZERO) - room_center_doc) * s)
}

pub fn room_camera_fit_fovy_for_corners(
    window_w: f32,
    window_h: f32,
    cam: CameraParams,
    corners_world: &[Vec3],
    margin_ndc: f32,
) -> CameraParams {
    renv::room_camera_fit_fovy_for_corners(window_w, window_h, cam, corners_world, margin_ndc)
}

pub fn room_camera_fit_clip_planes(cam: CameraParams, corners_world: &[Vec3]) -> CameraParams {
    renv::room_camera_fit_clip_planes(cam, corners_world)
}

/// Embedded-room camera with bounds-tight clip planes (hallway / shop / archive).
pub fn room_camera_with_room_clip_planes(
    mut cam: CameraParams,
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
) -> CameraParams {
    let corners = room_world_bounds_corners_centered(window_h, env_height_scale, cpu);
    if corners.is_empty() {
        return cam;
    }
    cam = room_camera_fit_clip_planes(cam, &corners);
    cam
}

/// Decoded room GLB (shop, hallway, …): shared layout for [`room_glb.wgsl`] and punctual uploads.
pub struct RoomGlbCpu {
    /// Compressed `.glb` bytes read from asset packs / loose files.
    pub packed_asset_bytes_read: u64,
    /// Decoded CPU payload bytes before room-environment mesh release.
    pub decoded_cpu_payload_bytes: u64,
    pub markers: FxHashMap<String, Mat4>,
    pub environment_primitives: Vec<RoomEnvPrimitiveCpu>,
    /// Set by [`release_room_environment_primitives_cpu`]. Keeps collision / rain /
    /// marker metadata without re-parsing the glTF on every [`with_shop_glb_cpu`] call.
    pub environment_primitives_released: bool,
    pub environment_bounds_doc: Option<RoomEnvironmentBounds>,
    pub marker_mesh_bounds_doc: FxHashMap<String, RoomEnvironmentBounds>,
    pub collision_meshes: Vec<RoomCollisionMesh>,
    pub embedded_perspective_camera: Option<RoomGlbEmbeddedCamera>,
    /// All embedded perspective cameras keyed by lowercase glTF node name (e.g. hallway `default` / `boss`).
    pub embedded_cameras_by_name: FxHashMap<String, RoomGlbEmbeddedCamera>,
    pub embedded_point_lights: Vec<RoomGlbEmbeddedPointLight>,
    pub embedded_spot_lights: Vec<RoomGlbEmbeddedSpotLight>,
    /// Invisible `rain_hit_*` shells for CPU rain splashes (main menu, etc.).
    pub rain_surface_meshes: Vec<RoomCollisionMesh>,
    /// [`rain_surface_meshes`] merged for per-drop raycasts (built at glTF load).
    /// `Arc` so per-frame accessors hand out a cheap handle instead of cloning the triangle soup.
    pub rain_surface_merged: Option<std::sync::Arc<RoomCollisionMesh>>,
    /// Named node bind poses captured during the glTF scene walk.
    pub node_bind_poses: FxHashMap<String, renv::RoomNodeBindPose>,
    /// Parsed glTF node TRS animation clips keyed by animation name.
    pub gltf_anim_library: crate::room_gltf_anim::RoomGltfAnimLibrary,
}

impl RoomGlbCpu {
    /// Merged `rain_hit_*` soup for CPU rain collision (see [`Self::rain_surface_merged`]).
    #[inline]
    pub fn rain_collision_mesh(&self) -> Option<&std::sync::Arc<RoomCollisionMesh>> {
        self.rain_surface_merged.as_ref()
    }

    /// glTF node local transform in **document** space (before [`room_env_model_matrix_from_cpu`]).
    #[inline]
    pub fn marker_node_transform_doc(&self, node_name: &str) -> Option<Mat4> {
        self.markers.get(node_name).copied()
    }

    /// Document-space AABB for the marker node's mesh (when decoded), for bounds / screen projection.
    #[inline]
    pub fn marker_mesh_bounds_doc_for(&self, node_name: &str) -> Option<&RoomEnvironmentBounds> {
        self.marker_mesh_bounds_doc.get(node_name)
    }
}

#[derive(Copy, Clone)]
struct RoomWalkHooks;

impl RoomEnvWalkHooks for RoomWalkHooks {
    fn is_marker(&self, name: &str) -> bool {
        is_marker_name(name)
    }

    fn mesh_policy(&self, name: &str) -> RoomMeshPolicy {
        if skip_room_env_mesh_for_node_name(name) {
            RoomMeshPolicy::SkipDrawCollisionIfMarker
        } else if is_shop_counter_button_node(name) {
            RoomMeshPolicy::EnvironmentDrawWithCollision
        } else {
            RoomMeshPolicy::EnvironmentDraw
        }
    }

    fn log_asset_label(&self) -> &'static str {
        "shop.glb"
    }
}

#[inline]
pub fn spawn_relic_marker_name(slot: usize) -> String {
    format!("shop_spawn_relic_{slot:02}")
}

#[inline]
pub fn player_relic_marker_name(slot: usize) -> String {
    format!("shop_player_relic_{slot:02}")
}

#[inline]
pub fn player_consumable_marker_name(slot: usize) -> String {
    format!("shop_player_consumable_{slot:02}")
}

/// glTF node name for the shop gold dish anchor (gameplay-style coin pile is centered here).
/// Blender exports often use snake_case; [`player_gold_dish_marker_translation`] also checks legacy PascalCase.
pub const PLAYER_GOLD_DISH_MARKER: &str = "player_gold_dish";

/// Document-space offset for the gold dish empty, same basis as [`marker_translation`].
#[inline]
pub fn player_gold_dish_marker_translation(cpu: &RoomGlbCpu) -> Option<Vec3> {
    marker_translation(cpu, PLAYER_GOLD_DISH_MARKER)
        .or_else(|| marker_translation(cpu, "PlayerGoldDish"))
}

fn is_marker_name(name: &str) -> bool {
    matches!(
        name,
        "exit_btn"
            | "restock_btn"
            | "journal_btn"
            | "Dish"
            | PLAYER_GOLD_DISH_MARKER
            | "PlayerGoldDish"
    ) || name.starts_with("shop_spawn_relic_")
        || name.starts_with("shop_player_relic_")
        || name.starts_with("shop_player_consumable_")
}

#[inline]
fn is_shop_counter_button_node(name: &str) -> bool {
    matches!(name, "exit_btn" | "restock_btn" | "journal_btn")
}

/// Environment draw skip: anchor nodes often have collision/helper meshes that should not render.
/// Button markers are excluded — their mesh is a visible control and may bind focus UI.
fn skip_room_env_mesh_for_node_name(name: &str) -> bool {
    name.starts_with("shop_spawn_relic_")
        || name.starts_with("shop_player_relic_")
        || name.starts_with("shop_player_consumable_")
}

/// Screen projection inputs for [`screen_rect_for_marker_mesh_bounds`].
pub struct MarkerScreenRectParams<'a> {
    pub win_w: f32,
    pub win_h: f32,
    pub cam: &'a crate::draw_cmd::CameraParams,
    pub env_height_scale: f32,
    pub cpu: &'a RoomGlbCpu,
    pub node_name: &'a str,
    pub min_rw: f32,
    pub min_rh: f32,
}

/// Project the eight corners of a named marker's decoded mesh AABB to screen pixels (same centering
/// / scale as the uploaded room). Returns `None` if bounds are missing. Clamps size to at least
/// `min_rw` × `min_rh` while keeping the projected center.
pub fn screen_rect_for_marker_mesh_bounds(p: &MarkerScreenRectParams<'_>) -> Option<[f32; 4]> {
    let bounds = p.cpu.marker_mesh_bounds_doc_for(p.node_name)?;
    let s = room_env_world_scale(p.win_h, p.env_height_scale);
    let center_doc = p
        .cpu
        .environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(Vec3::ZERO);
    let mut mn_x = f32::INFINITY;
    let mut mn_y = f32::INFINITY;
    let mut mx_x = f32::NEG_INFINITY;
    let mut mx_y = f32::NEG_INFINITY;
    for c in bounds.corners() {
        let world = (c - center_doc) * s;
        let (sx, sy) = p.cam.project_world_to_screen(p.win_w, p.win_h, world);
        mn_x = mn_x.min(sx);
        mn_y = mn_y.min(sy);
        mx_x = mx_x.max(sx);
        mx_y = mx_y.max(sy);
    }
    let cx = (mn_x + mx_x) * 0.5;
    let cy = (mn_y + mx_y) * 0.5;
    let rw = ((mx_x - mn_x).max(1.0)).max(p.min_rw);
    let rh = ((mx_y - mn_y).max(1.0)).max(p.min_rh);
    Some([cx - rw * 0.5, cy - rh * 0.5, rw, rh])
}

/// World-space unit-cube primitive matching a marker's decoded mesh AABB (same basis as
/// [`screen_rect_for_marker_mesh_bounds`]). Used by debug overlays (e.g. Button AABB Lab).
pub fn marker_mesh_bounds_reference_object3d(
    win_w: f32,
    win_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    node_name: &str,
    tint: [f32; 4],
) -> Option<crate::draw_cmd::Object3d> {
    use crate::draw_cmd::{Object3d, Object3dKind};
    use crate::primitive::{MaterialSpec, MeshId};
    use crate::world_space::object3d_pos_triple_for_world_center;

    let bounds = cpu.marker_mesh_bounds_doc_for(node_name)?;
    let s = room_env_world_scale(win_h, env_height_scale);
    let center_doc = cpu
        .environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(Vec3::ZERO);
    let center_world = (bounds.center() - center_doc) * s;
    let d = bounds.max - bounds.min;
    let extents = [(d.x * s).abs(), (d.y * s).abs(), (d.z * s).abs()];
    if extents[0] < 1e-6 || extents[1] < 1e-6 || extents[2] < 1e-6 {
        return None;
    }
    Some(Object3d {
        pos: object3d_pos_triple_for_world_center(win_w, win_h, center_world),
        extents,
        rotation: [0.0, 0.0, 0.0],
        color: tint,
        kind: Object3dKind::Primitive {
            shape: MeshId::Cube,
            material: MaterialSpec::plain(),
            pick_id: None,
            silhouette: false,
        },
        hover_target: 0.0,
        anim_id: 0,
    })
}

/// Shared glTF room walk used by [`load_shop_glb_from_bytes`] and [`crate::hallway_glb`].
pub fn load_room_glb_from_bytes(
    data: &[u8],
    import_err_ctx: &'static str,
    scene_err_ctx: &'static str,
    hooks: &impl RoomEnvWalkHooks,
) -> anyhow::Result<RoomGlbCpu> {
    thread_local! {
        static GLB_DECODE_BUMP: std::cell::RefCell<bumpalo::Bump> =
            std::cell::RefCell::new(bumpalo::Bump::new());
    }
    GLB_DECODE_BUMP.with(|b| b.borrow_mut().reset());

    let (document, buffers_vec, images) = gltf::import_slice(data).context(import_err_ctx)?;

    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .context(scene_err_ctx)?;

    let buffers: Vec<Vec<u8>> = buffers_vec.into_iter().map(|b| b.0).collect();
    let capped_images = renv::cap_room_gltf_images(&images);

    let mut markers = FxHashMap::default();
    let mut environment_primitives = Vec::new();
    let mut marker_mesh_bounds_doc = FxHashMap::default();
    let mut collision_meshes = Vec::new();
    let mut embedded_cameras = EmbeddedCameraHarvest::default();
    let mut embedded_point_lights = Vec::new();
    let mut embedded_spot_lights = Vec::new();
    let mut rain_surface_meshes = Vec::new();

    let mut node_bind_poses = FxHashMap::default();
    let mut texture_bake_cache = crate::tile_glb::TextureBakeCache::default();
    let mut walk_state = RoomEnvWalkState {
        candle_node_prefix: SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX,
        lantern_node_prefix: SHOP_GLTF_LANTERN_LIGHT_NODE_PREFIX,
        node_bind_poses: &mut node_bind_poses,
        markers: &mut markers,
        env_primitives: &mut environment_primitives,
        marker_mesh_bounds_doc: &mut marker_mesh_bounds_doc,
        collision_meshes: &mut collision_meshes,
        rain_surface_meshes: &mut rain_surface_meshes,
        embedded_cameras: &mut embedded_cameras,
        embedded_point_lights: &mut embedded_point_lights,
        embedded_spot_lights: &mut embedded_spot_lights,
        buffers: &buffers,
        capped_images: &capped_images,
        texture_bake_cache: &mut texture_bake_cache,
    };
    for node in scene.nodes() {
        walk_room_env_node(node, Mat4::IDENTITY, false, hooks, &mut walk_state)?;
    }

    let (embedded_perspective_camera, embedded_cameras_by_name) = embedded_cameras.into_parts();
    let environment_bounds_doc = room_environment_bounds(&environment_primitives);
    let gltf_anim_library = crate::room_gltf_anim::parse_gltf_anim_library(
        data,
        &node_bind_poses,
        hooks.log_asset_label(),
    );

    let rain_surface_merged =
        renv::RoomCollisionMesh::merge_rain_surfaces(&rain_surface_meshes).map(std::sync::Arc::new);
    if !embedded_spot_lights.is_empty() {
        log::error!(
            "{}: {} embedded spot light(s) in glTF — remove spot nodes; use programmatic \
             `SceneLighting::spot_lights` instead (punctual-only shadow path)",
            hooks.log_asset_label(),
            embedded_spot_lights.len(),
        );
    }
    let decoded_cpu_payload_bytes =
        crate::room_gpu_profile::count_cpu_payload(&environment_primitives).total_bytes();
    Ok(RoomGlbCpu {
        packed_asset_bytes_read: data.len() as u64,
        decoded_cpu_payload_bytes,
        markers,
        environment_primitives,
        environment_primitives_released: false,
        environment_bounds_doc,
        marker_mesh_bounds_doc,
        collision_meshes,
        embedded_perspective_camera,
        embedded_cameras_by_name,
        embedded_point_lights,
        embedded_spot_lights,
        rain_surface_meshes,
        rain_surface_merged,
        node_bind_poses,
        gltf_anim_library,
    })
}

pub fn load_shop_glb_from_bytes(data: &[u8]) -> anyhow::Result<RoomGlbCpu> {
    load_room_glb_from_bytes(
        data,
        "gltf::import_slice(shop.glb)",
        "shop.glb has no scenes",
        &RoomWalkHooks,
    )
}

/// `true` when `shop.glb` carries `KHR_lights_punctual` lights.
#[inline]
pub fn shop_glb_has_embedded_lights() -> bool {
    with_shop_glb_cpu(|opt| {
        opt.is_some_and(crate::room_gltf_punctual::room_glb_has_embedded_lights)
    })
}

/// glTF punctual points for the shop (candle flicker envelope).
pub fn shop_embedded_point_lights_runtime_tagged(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
    flame_time_s: f32,
    lamp_flicker: f32,
    candle_flicker_amp: f32,
) -> Vec<crate::room_gltf_punctual::EmbeddedPointLightRuntime> {
    with_shop_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_point_lights_runtime_tagged(
                cpu,
                w,
                h,
                env_h,
                tune,
                crate::room_gltf_punctual::RoomPunctualProfile::ShopCandles {
                    flame_time_s,
                    lamp_flicker,
                    flicker_amp: candle_flicker_amp,
                },
                "shop.glb",
            )
        })
        .unwrap_or_default()
    })
}

/// glTF punctual points for the shop (candle flicker envelope).
pub fn shop_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
    flame_time_s: f32,
    lamp_flicker: f32,
    candle_flicker_amp: f32,
) -> Vec<crate::wgpu_renderer::PointLight> {
    shop_embedded_point_lights_runtime_tagged(
        w,
        h,
        env_h,
        tune,
        flame_time_s,
        lamp_flicker,
        candle_flicker_amp,
    )
    .into_iter()
    .map(|t| t.light)
    .collect()
}

/// glTF spot lights for the shop room.
pub fn shop_embedded_spot_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<crate::wgpu_renderer::SpotLight> {
    with_shop_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_spot_lights_runtime(
                cpu, w, h, env_h, tune, "shop.glb",
            )
        })
        .unwrap_or_default()
    })
}

/// Shop camera from embedded GLB perspective camera, scaled like marker geometry.
#[inline]
pub fn shop_camera_from_glb_if_present(
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    with_shop_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        cpu.embedded_perspective_camera
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

/// Document-space marker origin minus environment AABB center (multiply by [`room_env_world_scale`]
/// for world space consistent with the centered shop model matrix).
pub fn marker_translation(cpu: &RoomGlbCpu, name: &str) -> Option<Vec3> {
    marker_translation_doc(&cpu.markers, cpu.environment_bounds_doc, name)
}

#[cfg(test)]
mod tests {
    /// Keep in sync with `gold_sign_body` / `gold_sign_hdr` in `shaders/room_glb.wgsl` `shop_shade`.
    fn gold_sign_body_fill_lum(albedo: [f32; 3], metallic: f32, ndotv: f32) -> f32 {
        let smoothstep = |e0: f32, e1: f32, x: f32| {
            let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let albedo_lum = 0.299 * albedo[0] + 0.587 * albedo[1] + 0.114 * albedo[2];
        let warm_gold_sign = albedo_lum > 0.45
            && albedo[0] > albedo[1] * 0.90
            && albedo[1] > albedo[2] * 1.20
            && albedo[0] > albedo[2] * 2.2;
        if !warm_gold_sign {
            return 0.0;
        }
        let gold_sign_ramp = smoothstep(0.45, 0.85, metallic);
        let scale = (0.22 + 0.40 * ndotv.powf(0.7)) * gold_sign_ramp;
        let rgb = [albedo[0] * scale, albedo[1] * scale, albedo[2] * scale];
        0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]
    }

    /// Keep in sync with the F0 gold-boost ramp in `shaders/room_glb.wgsl` `shop_shade`.
    fn gold_f0_boost(metallic: f32, albedo_lum: f32) -> f32 {
        let smoothstep = |e0: f32, e1: f32, x: f32| {
            let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let metal_ramp = smoothstep(0.45, 0.65, metallic);
        let dark_ramp = 1.0 - smoothstep(0.04, 0.16, albedo_lum);
        metal_ramp * dark_ramp
    }

    const ROOM_GLB_WGSL: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shaders/room_glb.wgsl"
    ));

    #[test]
    fn gold_f0_boost_is_continuous_near_old_threshold() {
        let metallic = 1.0;
        for &lum in &[0.05_f32, 0.065, 0.07, 0.075, 0.085, 0.09] {
            let b0 = gold_f0_boost(metallic, lum);
            let b1 = gold_f0_boost(metallic, lum + 0.005);
            assert!(
                (b0 - b1).abs() < 0.1,
                "boost should not cliff at lum={lum}: {b0} vs {b1}"
            );
        }
    }

    #[test]
    fn gold_f0_boost_dark_gold_front_face_is_strong() {
        for lum in [0.05_f32, 0.065, 0.075, 0.085, 0.09] {
            let boost = gold_f0_boost(1.0, lum);
            assert!(
                boost > 0.5,
                "dark gold front face should get strong F0 boost at lum={lum}, got {boost}"
            );
        }
    }

    #[test]
    fn gold_f0_boost_endpoints_untouched() {
        assert_eq!(gold_f0_boost(0.0, 0.05), 0.0);
        assert_eq!(gold_f0_boost(0.0, 0.09), 0.0);
        assert_eq!(gold_f0_boost(1.0, 0.16), 0.0);
        assert_eq!(gold_f0_boost(1.0, 0.20), 0.0);
    }

    #[test]
    fn shop_gold_text_has_bright_warm_albedo_not_alpha_mask() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/3d/Shop.glb");
        let data = std::fs::read(&path).expect("Shop.glb");
        let cpu = crate::room_glb::load_shop_glb_from_bytes(&data).expect("load shop");
        for name in ["Text", "SHOP"] {
            let ep = cpu
                .environment_primitives
                .iter()
                .find(|ep| ep.gltf_node_name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing node {name}"));
            let (pix, w, h) = ep
                .mesh
                .albedo_rgba
                .as_deref()
                .expect("Gold text should have base color texture");
            let sample = |u: f32, v: f32| -> [f32; 3] {
                let wf = *w as f32;
                let hf = *h as f32;
                let x = ((u * wf - 0.5).clamp(0.0, wf - 1.0)) as u32;
                let y = ((v * hf - 0.5).clamp(0.0, hf - 1.0)) as u32;
                let i = ((y * w + x) * 4) as usize;
                let c = &pix[i..i + 4];
                [
                    c[0] as f32 / 255.0,
                    c[1] as f32 / 255.0,
                    c[2] as f32 / 255.0,
                ]
            };
            for v in &ep.mesh.vertices {
                let rgb = sample(v.uv[0], v.uv[1]);
                let lum = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2];
                assert!(
                    lum > 0.35,
                    "{name}: expected bright gold albedo at vertex UV, got lum={lum:.3}"
                );
            }
        }
    }

    #[test]
    fn gold_sign_body_fill_strong_for_shop_gold_facing_camera() {
        // Representative decoded Gold_BaseColor sample (~linear sRGB after upload).
        let albedo = [0.88_f32, 0.72, 0.32];
        let fill = gold_sign_body_fill_lum(albedo, 1.0, 1.0);
        assert!(
            fill > 0.25,
            "camera-facing shop gold should get visible body fill, got {fill}"
        );
    }

    #[test]
    fn gold_sign_body_fill_off_for_dielectric_wood() {
        let wood = [0.35_f32, 0.22, 0.12];
        assert_eq!(gold_sign_body_fill_lum(wood, 0.0, 1.0), 0.0);
        assert_eq!(gold_sign_body_fill_lum(wood, 1.0, 1.0), 0.0);
    }

    #[test]
    fn gold_sign_hdr_tracks_room_linear_exposure_base() {
        let albedo = [0.88_f32, 0.72, 0.32];
        let body = gold_sign_body_fill_lum(albedo, 1.0, 1.0);
        let base_default = super::ROOM_GLB_LINEAR_EXPOSURE_BASE;
        let base_dark = 1e-4_f32;
        let hdr_default = body * base_default / base_default;
        let hdr_dark = body * base_dark / base_default;
        assert!((hdr_default - body).abs() < 1e-6);
        assert!(
            hdr_dark < body * 0.1,
            "crushed exposure base should dim gold fill, got {hdr_dark} vs body {body}"
        );
    }

    #[test]
    fn room_glb_wgsl_uses_smooth_gold_ramps_not_hard_cliffs() {
        assert!(
            !ROOM_GLB_WGSL.contains("albedo_lum < 0.07"),
            "hard F0 cliff should be replaced by smooth ramps"
        );
        assert!(
            !ROOM_GLB_WGSL.contains("albedo_lum < 0.08"),
            "hard hemi cliff should be replaced by smooth ramps"
        );
        assert!(
            ROOM_GLB_WGSL.contains("smoothstep(0.45, 0.65, metallic)"),
            "F0 gold-boost ramp missing from room_glb.wgsl"
        );
        assert!(
            ROOM_GLB_WGSL.contains("gold_sign_body"),
            "shop gold signage body fill missing from room_glb.wgsl"
        );
        assert!(
            ROOM_GLB_WGSL.contains("gold_sign_hdr"),
            "shop gold signage exposure-scaled fill missing from room_glb.wgsl"
        );
        assert!(
            ROOM_GLB_WGSL.contains("warm_gold_sign"),
            "warm gold signage detection missing from room_glb.wgsl"
        );
        assert!(
            ROOM_GLB_WGSL.contains("GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE"),
            "main-menu moon feature flag missing from room_glb.wgsl"
        );
        assert!(
            ROOM_GLB_WGSL.contains("GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL"),
            "archive decal feature flag missing from room_glb.wgsl"
        );
        assert!(
            ROOM_GLB_WGSL.contains("exponential_height_fog_alpha"),
            "analytic exponential height fog helper missing from room_glb.wgsl"
        );
        assert!(
            ROOM_GLB_WGSL.contains("room_height_fog_params"),
            "room height fog uniform missing from room_glb.wgsl"
        );
        assert!(
            !ROOM_GLB_WGSL.contains("pbr.emissive_factor.w >"),
            "room_glb.wgsl should not overload emissive_factor.w as a feature tag"
        );
        assert!(
            !ROOM_GLB_WGSL.contains("abs(in.v_color.a - 2.0)"),
            "room_glb.wgsl should not treat COLOR_0.a as an archive decal feature tag"
        );
        assert!(
            !ROOM_GLB_WGSL.contains("abs(in.v_color.a - 4.0)"),
            "room_glb.wgsl should not treat COLOR_0.a as a hallway tint feature tag"
        );
    }

    #[test]
    fn released_environment_is_stale_for_gpu_retry_not_corrupt_reload() {
        let mut cpu = super::RoomGlbCpu {
            packed_asset_bytes_read: 0,
            decoded_cpu_payload_bytes: 0,
            markers: Default::default(),
            environment_primitives: Vec::new(),
            environment_primitives_released: true,
            environment_bounds_doc: Some(crate::room_env_gltf::RoomEnvironmentBounds {
                min: glam::Vec3::ZERO,
                max: glam::Vec3::ONE,
            }),
            marker_mesh_bounds_doc: Default::default(),
            collision_meshes: Vec::new(),
            embedded_perspective_camera: None,
            embedded_cameras_by_name: Default::default(),
            embedded_point_lights: Vec::new(),
            embedded_spot_lights: Vec::new(),
            rain_surface_meshes: Vec::new(),
            rain_surface_merged: None,
            node_bind_poses: Default::default(),
            gltf_anim_library: Default::default(),
        };
        assert!(super::room_glb_cpu_stale_environment_for_gpu_upload(&cpu));
        assert!(!super::room_glb_cpu_needs_environment_mesh_reload(&cpu));
        cpu.environment_primitives_released = false;
        assert!(super::room_glb_cpu_needs_environment_mesh_reload(&cpu));
        assert!(!super::room_glb_cpu_stale_environment_for_gpu_upload(&cpu));
    }
}
