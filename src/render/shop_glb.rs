//! Load [`Shop.glb`](../../../assets/Shop.glb): named empties/meshes for UI anchors + merged environment geometry.
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
//! decode time so it does not draw, but it is still decoded into **[`ShopCollisionMesh`]** triangle
//! soups for cursor ray picking (`pick_shop_object`). **Shop buttons** (`exit_btn`, `restock_btn`,
//! `journal_btn`) still record marker transforms **and** decode their meshes for drawing.
//!
//! ## Materials
//! Each primitive uses glTF PBR **base color texture** (if present) and multiplies by
//! **`baseColorFactor`** on the CPU. Factor-only materials become a 1×1 uploaded texture.
//! **Normal maps** (`material.normalTexture`) are decoded as linear RGBA; **`scale`** is baked
//! into texels. Tangents come from the glTF `TANGENT` attribute when present, otherwise from
//! [`crate::render::tile_glb::compute_vertex_tangents`] using the normal map TEXCOORD when it
//! differs from base color. Metallic–roughness, emissive, alpha modes, `COLOR_0`, and glTF sampler
//! settings follow [`crate::render::tile_glb::LoadedPrimitive`] (shared with `Tile.glb`).
//!
//! ## Export (Blender / glTF)
//! Ship **`Shop.glb` without Draco** (`KHR_draco_mesh_compression`). This crate uses
//! [`gltf::import_slice`](https://docs.rs/gltf), which does not decode Draco — compressed files fail
//! validation (`accessor.bufferView: Missing data`, unsupported extension).
//!
//! **Blender parity:** In the glTF 2.0 exporter use **Lighting Mode → Standard** (Khronos cd / lx
//! units). See the [Blender glTF manual](https://www.blender.org/manual/en/latest/addons/import_export/scene_gltf2.html)
//! (*Data → Lighting*). Re-export after changing light units so authored intensities match
//! [`KHR_lights_punctual`](https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_lights_punctual/README.md).
//!
//! **Validation:** Compare punctual brightness and cones in a reference viewer (e.g.
//! [Don McCurdy’s glTF Viewer](https://gltf-viewer.donmccurdy.com/)) against this build; differences
//! vs the Blender viewport are usually exposure or scene scale, not JSON alone.
//!
//! ## Scale & framing
//! Environment vertices are **centered** using the axis-aligned bounds of all shop mesh geometry:
//! the GPU model is `translate(-center * s) * uniformScale(s)` with `s = window_h * height_scale`, so
//! the room’s geometric center sits at world origin and stays on-screen as resolution changes.
//! The perspective camera (embedded or fallback) is offset the same way; vertical FOV is **raised**
//! only when needed so the bounds’ corners stay inside the frustum at the current aspect ratio.
//! Default multiplier is [`SHOP_ENV_HEIGHT_SCALE`]; Debug → Tuning → **Shop Env & Lighting…**
//! overrides height scale and [`ShopEnvLightingTune`] fields live (typical height range `0.001`–`2.0`).
//!
//! ## Optional perspective camera
//! If the default scene contains a **perspective** camera node, the shop uses it for
//! [`crate::render::draw_cmd::CameraParams`] (eye / target / up / vertical FOV). Transforms are read
//! in glTF camera convention (−Z forward, +Y up); positions are scaled by [`shop_env_world_scale`]
//! like marker geometry. If multiple cameras exist, a node named `ShopCamera`, `shop_camera`, or
//! `Camera` wins; otherwise the first perspective camera in depth-first order is used. Orthographic
//! cameras are ignored (hardcoded fallback camera applies).
//!
//! ## `KHR_lights_punctual`
//! **Point** and **spot** lights on scene nodes drive shop lighting when present: hardcoded lamp +
//! fill point lights are omitted so only glTF punctual lights apply (hover highlights may still add
//! extras). **Directional** lights are skipped. With embedded lights, the room draws through
//! `shop_glb.wgsl`: inverse-square attenuation (Khronos range window),
//! metallic–roughness, ACES (fitted) tonemap, and linear HDR exposure:
//! [`SHOP_ENV_LINEAR_EXPOSURE_BASE`] × debug tune (see [`SHOP_ENV_LINEAR_EXPOSURE`]) before tonemap;
//! [`SHOP_ENV_AMBIENT_SCALE`] defaults to `0` for this interior.
//! glTF punctual intensity is scaled by [`SHOP_GLTF_LIGHT_INTENSITY_SCALE`] (default `1`). Shop punctual
//! points use a separate uniform buffer, bound as group 1 binding 0 for [`shop_glb.wgsl`] and binding 2
//! for [`lit_mesh.wgsl`] (inverse-square on props; stays within WebGPU `max_bind_groups` on Metal).
//! Punctual lights on nodes whose names start with [`SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX`] use
//! [`SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL`] for a warm candle read; other lights keep glTF-authored color.
//! `range` maps to glTF max distance (`0` = infinite).
//!
//! Shared glTF room decode (meshes, lights, cameras, collision) lives in [`crate::render::room_env_gltf`].

use std::collections::HashMap;
use std::sync::RwLock;

use crate::render::draw_cmd::CameraParams;
use crate::render::room_env_gltf::{
    self as renv, EmbeddedCameraHarvest, RoomEnvWalkHooks, RoomMeshPolicy, marker_translation_doc,
    room_camera_fit_fovy_for_corners, room_env_model_matrix_from_bounds_doc,
    room_world_bounds_corners_centered, walk_room_env_node,
};
use crate::render::tile_glb::release_loaded_primitive_gpu_source_buffers;
use anyhow::Context;
use glam::{Mat4, Vec3};

enum ShopGlbCache {
    Uninit,
    Ready(Option<RoomGlbCpu>),
}

static SHOP_GLB_CPU: RwLock<ShopGlbCache> = RwLock::new(ShopGlbCache::Uninit);

fn ensure_shop_glb_loaded() {
    let mut w = SHOP_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if !matches!(*w, ShopGlbCache::Uninit) {
        return;
    }
    let ready = if let Some(file) = crate::asset_path::get("Shop.glb") {
        match load_shop_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::debug!(
                    "Shop.glb: {} marker node(s), {} draw primitive(s), {} collision mesh(es)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                    cpu.collision_meshes.len(),
                );
                if cpu.embedded_perspective_camera.is_some()
                    || !cpu.embedded_point_lights.is_empty()
                    || !cpu.embedded_spot_lights.is_empty()
                {
                    log::debug!(
                        "Shop.glb scene extras: perspective_camera={} point_lights={} spot_lights={}",
                        cpu.embedded_perspective_camera.is_some(),
                        cpu.embedded_point_lights.len(),
                        cpu.embedded_spot_lights.len(),
                    );
                    if !cpu.embedded_point_lights.is_empty() || !cpu.embedded_spot_lights.is_empty()
                    {
                        log::debug!(
                            "Shop.glb punctual lights: re-export from Blender glTF with Lighting Mode **Standard** (cd/lx); validate in https://gltf-viewer.donmccurdy.com/"
                        );
                    }
                }
                Some(cpu)
            }
            Err(e) => {
                let msg = format!("{e:#}");
                log::error!("Shop.glb failed to load: {msg}");
                if msg.contains("KHR_draco_mesh_compression") {
                    log::warn!(
                        "Re-export Shop.glb with Draco compression disabled (Blender glTF: turn off mesh compression / Draco)."
                    );
                }
                None
            }
        }
    } else {
        log::warn!("Shop.glb not embedded; using PNG storeroom backdrop");
        None
    };
    *w = ShopGlbCache::Ready(ready);
}

/// Read-only access to decoded shop data (markers, lights, collision, …).  
/// Do not call [`release_shop_environment_cpu_sources_after_gpu_upload`] from inside `f` (deadlock).
pub fn with_shop_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_shop_glb_loaded();
    let g = SHOP_GLB_CPU.read().unwrap_or_else(|e| e.into_inner());
    match &*g {
        ShopGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        ShopGlbCache::Ready(None) => f(None),
        ShopGlbCache::Uninit => unreachable!(),
    }
}

/// Drops environment mesh + decoded texture RAM after [`crate::render::wgpu_renderer::WgpuRenderer`]
/// has uploaded shop draws to the GPU. Safe to call once at init; no-op if shop failed to load.
pub fn release_room_environment_primitives_cpu(cpu: &mut RoomGlbCpu) {
    for env in &mut cpu.environment_primitives {
        release_loaded_primitive_gpu_source_buffers(&mut env.mesh);
    }
    cpu.environment_primitives.shrink_to_fit();
}

pub fn release_shop_environment_cpu_sources_after_gpu_upload() {
    let mut g = SHOP_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if let ShopGlbCache::Ready(Some(cpu)) = &mut *g {
        release_room_environment_primitives_cpu(cpu);
    }
}

/// Default height multiplier for [`shop_env_world_scale`] when no debug override is active.
pub const SHOP_ENV_HEIGHT_SCALE: f32 = 1.0;

/// Multiplies glTF punctual **intensity** before upload (document-space inverse-square; see
/// `decal_atlas_uv.y` / `SsrGlobals.shop_punctual`). Default `1` uses authored intensities.
pub const SHOP_GLTF_LIGHT_INTENSITY_SCALE: f32 = 0.6;

/// Linear HDR gain for **shop** only: `2^-9` ≈ Don McCurdy glTF viewer exposure **−9** (EV on linear HDR).
/// Multiplied with [`ShopEnvLightingTune::linear_exposure`] and written to `shop_glb` / shop `lit_mesh` path.
pub const SHOP_ENV_LINEAR_EXPOSURE_BASE: f32 = 1.0 / 512.0; // 2^-9

/// Extra multiplier on room glTF emissive (`CameraUniform.decal_atlas_uv.z`), after
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

/// Default **tuning** multiplier for linear HDR (debug overlay); shop applies [`SHOP_ENV_LINEAR_EXPOSURE_BASE`]
/// on top. Table scenes use this value alone (no shop base).
pub const SHOP_ENV_LINEAR_EXPOSURE: f32 = 1.0;

/// Linear HDR multiplier for tile-pack celebration (`Scene::TilePackCelebration`): no
/// [`crate::render::draw_cmd::DrawCmd::ShopEnvironment`], but showcase tiles still use shop-style
/// punctual lights — without this, `tile_hdr_tonemap` falls back to `linear_hdr ≈ 1` and faces clip.
/// Between full shop (`×`[`SHOP_ENV_LINEAR_EXPOSURE_BASE`]) and isolation showcase.
/// `lit_mesh` (pack box) shares this with showcase tiles; too low reads as black with zero ambient.
pub const TILE_PACK_CELEBRATION_HDR_LINEAR_EXPOSURE: f32 = 1.0 / 3.8;

/// Floor for `hdr_tonemap.z` → `lit_mesh` `felt.w` hemispheric fill (`× 0.08` in shader) on pack scenes
/// when [`SHOP_ENV_AMBIENT_SCALE`] is 0.
pub const TILE_PACK_CELEBRATION_LIT_MESH_AMBIENT_MIN: f32 = 0.62;

/// Shop [`ItemInspectScene`] `lit_mesh` path: synthetic inspect lights only (no GLB punctual).
/// [`SHOP_ENV_LINEAR_EXPOSURE_BASE`] is too dark; [`GAMEPLAY_TABLE_HDR_LINEAR_MUL`] reads blown out.
pub const SHOP_INSPECT_LIT_MESH_HDR_LINEAR_MUL: f32 = 1.0 / 52.0;

/// Hemispheric fill term in `lit_mesh.wgsl` (`felt.w * 0.08` before ACES). Keep below
/// [`GAMEPLAY_TABLE_AMBIENT_MIN`] so inspect does not wash out like the table.
pub const SHOP_INSPECT_LIT_MESH_AMBIENT: f32 = 0.29;

/// Storeroom linear HDR during shop inspect = subject linear × this. Using the ratio of
/// legacy shop crush ([`SHOP_ENV_LINEAR_EXPOSURE_BASE`]) to subject inspect gain
/// ([`SHOP_INSPECT_LIT_MESH_HDR_LINEAR_MUL`]) ties both to the same tuning multiplier and
/// composite path (SDR and HDR swapchains share `tonemap_composite.wgsl`).
pub const SHOP_INSPECT_ENV_VS_LIT_LINEAR: f32 =
    SHOP_ENV_LINEAR_EXPOSURE_BASE / SHOP_INSPECT_LIT_MESH_HDR_LINEAR_MUL;

/// Room hemispheric fill vs subject `hdr_tonemap.z` during inspect (`tile_3d.wgsl`).
pub const SHOP_INSPECT_ENV_VS_LIT_AMBIENT: f32 = 0.45;

/// Item inspect disables GLB punctual and drives `shop_glb.wgsl` via `tile_seed` / ambient.
/// The storeroom BRDF stacks different radiance than `lit_mesh`; without this boost the room reads black.
pub const SHOP_INSPECT_STOREROOM_GLB_TILE_SEED_MUL: f32 = 12.0;

/// Hemispheric fill in `shop_glb.wgsl` (`decal_atlas_uv.x`).
pub const SHOP_ENV_AMBIENT_SCALE: f32 = 0.0;

/// Lower bound for `hdr_tonemap.z` on candle-key **table** scenes (`gameplay`, `tutorial`,
/// `pick_blind`, `collection`). Shop keeps authored ambient only. Tiles and `lit_mesh` add
/// `albedo * scale * 0.08` before ACES — without this, shadowed areas read as pure black and
/// lit areas clip warm, which feels flat and hyper-saturated.
pub const GAMEPLAY_TABLE_AMBIENT_MIN: f32 = 0.52;

/// Linear HDR multiplier for table scenes only (after [`ShopEnvLightingTune::linear_exposure`];
/// shop still applies [`SHOP_ENV_LINEAR_EXPOSURE_BASE`]). Slightly <1 reins in peak energy
/// before ACES so highlights retain separation from midtones.
pub const GAMEPLAY_TABLE_HDR_LINEAR_MUL: f32 = 0.88;

/// Applied in `lit_mesh.wgsl` as the punctual buffer `extras.w` when
/// [`crate::render::draw_cmd::SceneLighting::embedded_gltf_punctual`] is set (`shop_glb.wgsl` ignores it).
/// Defaults to `1` so embedded punctual lights match the room; debug tuning may lower it.
pub const SHOP_LIT_MESH_GLTF_PUNCTUAL_SCALE: f32 = 0.55;

/// glTF **node** name prefix for punctual lights that should read as warm candles (`light_candle_00`, …).
pub const SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX: &str = "light_candle_";

/// Linear RGB multiplier for punctual lights on nodes matching [`SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX`].
/// Warm shift for candle reads; other lights keep glTF linear RGB.
pub const SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL: [f32; 3] = [1.0, 0.91, 0.74];

/// Runtime shop lighting matching the `SHOP_*` source constants. Carried on [`DrawCtx`](crate::scenes::DrawCtx)
/// and editable from the debug overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShopEnvLightingTune {
    pub gltf_light_intensity_scale: f32,
    pub linear_exposure: f32,
    pub ambient_scale: f32,
    pub lit_mesh_gltf_punctual_scale: f32,
    /// Room glTF emissive strength ([`SHOP_GLTF_EMISSIVE_SCALE`] default).
    pub gltf_emissive_scale: f32,
    pub candle_light_color_mul: [f32; 3],
}

impl Default for ShopEnvLightingTune {
    fn default() -> Self {
        Self::SOURCE_DEFAULTS
    }
}

impl ShopEnvLightingTune {
    pub const SOURCE_DEFAULTS: Self = Self {
        gltf_light_intensity_scale: SHOP_GLTF_LIGHT_INTENSITY_SCALE,
        linear_exposure: SHOP_ENV_LINEAR_EXPOSURE,
        ambient_scale: SHOP_ENV_AMBIENT_SCALE,
        lit_mesh_gltf_punctual_scale: SHOP_LIT_MESH_GLTF_PUNCTUAL_SCALE,
        gltf_emissive_scale: SHOP_GLTF_EMISSIVE_SCALE,
        candle_light_color_mul: SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL,
    };
}

// --- Stable `Shop*` names (shared decode in `room_env_gltf`) ---
pub type ShopEnvPrimitiveCpu = renv::RoomEnvPrimitiveCpu;
pub type ShopCollisionMesh = renv::RoomCollisionMesh;
pub type ShopEnvironmentBounds = renv::RoomEnvironmentBounds;
pub type ShopGlbEmbeddedPointLight = renv::RoomGltfEmbeddedPointLight;
pub type ShopGlbEmbeddedSpotLight = renv::RoomGltfEmbeddedSpotLight;
pub type ShopGlbEmbeddedCamera = renv::RoomGltfEmbeddedCamera;

pub(crate) use crate::render::room_env_gltf::room_environment_bounds;
pub use crate::render::room_env_gltf::glb_punctual_range_world_upload;

#[inline]
pub fn shop_env_world_scale(window_h: f32, height_scale: f32) -> f32 {
    renv::room_env_world_scale(window_h, height_scale)
}

/// `translate(-center_doc * s) * uniformScale(s)` — matches centered room mesh + picking.
#[allow(dead_code)]
#[inline]
pub fn shop_env_model_matrix(window_h: f32, height_scale: f32, center_doc: Vec3) -> Mat4 {
    renv::room_env_model_matrix(window_h, height_scale, center_doc)
}

#[inline]
pub fn shop_env_model_matrix_from_cpu(window_h: f32, height_scale: f32, cpu: &RoomGlbCpu) -> Mat4 {
    room_env_model_matrix_from_bounds_doc(window_h, height_scale, cpu.environment_bounds_doc)
}

pub fn shop_world_bounds_corners_centered(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
) -> Vec<Vec3> {
    let Some(bounds) = cpu.environment_bounds_doc else {
        return Vec::new();
    };
    room_world_bounds_corners_centered(bounds, window_h, env_height_scale)
}

pub fn shop_camera_fit_fovy_for_corners(
    window_w: f32,
    window_h: f32,
    cam: CameraParams,
    corners_world: &[Vec3],
    margin_ndc: f32,
) -> CameraParams {
    room_camera_fit_fovy_for_corners(window_w, window_h, cam, corners_world, margin_ndc)
}

/// Decoded room GLB (shop, hallway, …): shared layout for [`shop_glb.wgsl`] and punctual uploads.
pub struct RoomGlbCpu {
    pub markers: HashMap<String, Mat4>,
    pub environment_primitives: Vec<ShopEnvPrimitiveCpu>,
    pub environment_bounds_doc: Option<ShopEnvironmentBounds>,
    pub marker_mesh_bounds_doc: HashMap<String, ShopEnvironmentBounds>,
    pub collision_meshes: Vec<ShopCollisionMesh>,
    pub embedded_perspective_camera: Option<ShopGlbEmbeddedCamera>,
    pub embedded_point_lights: Vec<ShopGlbEmbeddedPointLight>,
    pub embedded_spot_lights: Vec<ShopGlbEmbeddedSpotLight>,
}

/// Back-compat name — shop and hallway both decode to [`RoomGlbCpu`].
#[allow(dead_code)]
pub type ShopGlbCpu = RoomGlbCpu;

#[derive(Copy, Clone)]
struct ShopRoomWalkHooks;

impl RoomEnvWalkHooks for ShopRoomWalkHooks {
    fn is_marker(&self, name: &str) -> bool {
        is_marker_name(name)
    }

    fn mesh_policy(&self, name: &str) -> RoomMeshPolicy {
        if skip_shop_env_mesh_for_node_name(name) {
            RoomMeshPolicy::SkipDrawCollisionIfMarker
        } else if is_shop_counter_button_node(name) {
            RoomMeshPolicy::EnvironmentDrawWithCollision
        } else {
            RoomMeshPolicy::EnvironmentDraw
        }
    }

    fn log_asset_label(&self) -> &'static str {
        "Shop.glb"
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
fn skip_shop_env_mesh_for_node_name(name: &str) -> bool {
    name.starts_with("shop_spawn_relic_")
        || name.starts_with("shop_player_relic_")
        || name.starts_with("shop_player_consumable_")
}

/// Project the eight corners of a named marker's decoded mesh AABB to screen pixels (same centering
/// / scale as the uploaded room). Returns `None` if bounds are missing. Clamps size to at least
/// `min_rw` × `min_rh` while keeping the projected center.
pub fn screen_rect_for_marker_mesh_bounds(
    win_w: f32,
    win_h: f32,
    cam: &crate::render::draw_cmd::CameraParams,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    node_name: &str,
    min_rw: f32,
    min_rh: f32,
) -> Option<[f32; 4]> {
    let bounds = cpu.marker_mesh_bounds_doc.get(node_name)?;
    let s = shop_env_world_scale(win_h, env_height_scale);
    let center_doc = cpu
        .environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(Vec3::ZERO);
    let mut mn_x = f32::INFINITY;
    let mut mn_y = f32::INFINITY;
    let mut mx_x = f32::NEG_INFINITY;
    let mut mx_y = f32::NEG_INFINITY;
    for c in bounds.corners() {
        let world = (c - center_doc) * s;
        let (sx, sy) = cam.project_world_to_screen(win_w, win_h, world);
        mn_x = mn_x.min(sx);
        mn_y = mn_y.min(sy);
        mx_x = mx_x.max(sx);
        mx_y = mx_y.max(sy);
    }
    let cx = (mn_x + mx_x) * 0.5;
    let cy = (mn_y + mx_y) * 0.5;
    let rw = ((mx_x - mn_x).max(1.0)).max(min_rw);
    let rh = ((mx_y - mn_y).max(1.0)).max(min_rh);
    Some([cx - rw * 0.5, cy - rh * 0.5, rw, rh])
}

/// Shared glTF room walk used by [`load_shop_glb_from_bytes`] and [`crate::render::hallway_glb`].
pub fn load_room_glb_from_bytes(
    data: &[u8],
    import_err_ctx: &'static str,
    scene_err_ctx: &'static str,
    hooks: &impl RoomEnvWalkHooks,
) -> anyhow::Result<RoomGlbCpu> {
    let (document, buffers_vec, images) = gltf::import_slice(data).context(import_err_ctx)?;

    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .context(scene_err_ctx)?;

    let buffers: Vec<Vec<u8>> = buffers_vec.into_iter().map(|b| b.0).collect();

    let mut markers = HashMap::new();
    let mut environment_primitives = Vec::new();
    let mut marker_mesh_bounds_doc = HashMap::new();
    let mut collision_meshes = Vec::new();
    let mut embedded_cameras = EmbeddedCameraHarvest::default();
    let mut embedded_point_lights = Vec::new();
    let mut embedded_spot_lights = Vec::new();

    for node in scene.nodes() {
        walk_room_env_node(
            node,
            Mat4::IDENTITY,
            hooks,
            SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX,
            &mut markers,
            &mut environment_primitives,
            &mut marker_mesh_bounds_doc,
            &mut collision_meshes,
            &mut embedded_cameras,
            &mut embedded_point_lights,
            &mut embedded_spot_lights,
            &buffers,
            &images,
        )?;
    }

    let embedded_perspective_camera = embedded_cameras.pick();
    let environment_bounds_doc = room_environment_bounds(&environment_primitives);

    Ok(RoomGlbCpu {
        markers,
        environment_primitives,
        environment_bounds_doc,
        marker_mesh_bounds_doc,
        collision_meshes,
        embedded_perspective_camera,
        embedded_point_lights,
        embedded_spot_lights,
    })
}

pub fn load_shop_glb_from_bytes(data: &[u8]) -> anyhow::Result<RoomGlbCpu> {
    load_room_glb_from_bytes(
        data,
        "gltf::import_slice(Shop.glb)",
        "Shop.glb has no scenes",
        &ShopRoomWalkHooks,
    )
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

/// Document-space marker origin minus environment AABB center (multiply by [`shop_env_world_scale`]
/// for world space consistent with the centered shop model matrix).
pub fn marker_translation(cpu: &RoomGlbCpu, name: &str) -> Option<Vec3> {
    marker_translation_doc(&cpu.markers, cpu.environment_bounds_doc, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Documents how many environment primitives carry glTF emissive; re-run after
    /// authoring lamps in Blender so the count reflects exported `emissiveTexture` / factor.
    #[test]
    fn shop_glb_emissive_material_summary() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/Shop.glb");
        let data = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("skip shop_glb_emissive_material_summary: {path:?}: {e}");
                return;
            }
        };
        let cpu = load_shop_glb_from_bytes(&data).expect("Shop.glb decode");
        let mut with_tex = 0usize;
        let mut with_factor = 0usize;
        for ep in &cpu.environment_primitives {
            let m = &ep.mesh;
            if m.emissive_rgba.is_some() {
                with_tex += 1;
            }
            let f = m.emissive_factor;
            if f[0] > 1e-5 || f[1] > 1e-5 || f[2] > 1e-5 {
                with_factor += 1;
            }
        }
        eprintln!(
            "Shop.glb: {} env primitive(s), {} with emissive texture, {} with non-zero emissive factor",
            cpu.environment_primitives.len(),
            with_tex,
            with_factor
        );
    }
}
