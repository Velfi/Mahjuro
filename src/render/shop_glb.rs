//! Load [`Shop.glb`](../../../assets/Shop.glb): named empties/meshes for UI anchors + merged environment geometry.
//!
//! Marker object names (Blender object names → glTF node names):
//! - `exit_btn`, `restock_btn`, `journal_btn`
//! - [`PLAYER_GOLD_DISH_MARKER`] — origin for the procedural gold coin pile (place at the dish floor).
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

use std::collections::HashMap;
use std::sync::RwLock;

use crate::render::draw_cmd::CameraParams;
use crate::render::gltf_helpers::{apply_texture_transform, sampler_cpu_from_material};
use crate::render::tile_glb::{
    GltfAlphaMode, LoadedPrimitive, Vertex3dTex, compute_vertex_tangents, gltf_image_to_rgba8,
    multiply_rgba8_by_factor, release_loaded_primitive_gpu_source_buffers, solid_albedo_rgba8,
};
use anyhow::Context;
use glam::{Mat4, Vec3, Vec4};

enum ShopGlbCache {
    Uninit,
    Ready(Option<ShopGlbCpu>),
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
                log::info!(
                    "Shop.glb: {} marker node(s), {} draw primitive(s), {} collision mesh(es)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                    cpu.collision_meshes.len(),
                );
                if cpu.embedded_perspective_camera.is_some()
                    || !cpu.embedded_point_lights.is_empty()
                    || !cpu.embedded_spot_lights.is_empty()
                {
                    log::info!(
                        "Shop.glb scene extras: perspective_camera={} point_lights={} spot_lights={}",
                        cpu.embedded_perspective_camera.is_some(),
                        cpu.embedded_point_lights.len(),
                        cpu.embedded_spot_lights.len(),
                    );
                    if !cpu.embedded_point_lights.is_empty() || !cpu.embedded_spot_lights.is_empty() {
                        log::info!(
                            "Shop.glb punctual lights: re-export from Blender glTF with Lighting Mode **Standard** (cd/lx); validate in https://gltf-viewer.donmccurdy.com/"
                        );
                    }
                }
                Some(cpu)
            }
            Err(e) => {
                let msg = format!("{e:#}");
                log::warn!("Shop.glb failed to load: {msg}");
                if msg.contains("KHR_draco_mesh_compression") {
                    log::warn!(
                        "Re-export Shop.glb with Draco compression disabled (Blender glTF: turn off mesh compression / Draco)."
                    );
                }
                None
            }
        }
    } else {
        log::debug!("Shop.glb not embedded; using PNG storeroom backdrop");
        None
    };
    *w = ShopGlbCache::Ready(ready);
}

/// Read-only access to decoded shop data (markers, lights, collision, …).  
/// Do not call [`release_shop_environment_cpu_sources_after_gpu_upload`] from inside `f` (deadlock).
pub fn with_shop_glb_cpu<R>(f: impl FnOnce(Option<&ShopGlbCpu>) -> R) -> R {
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
pub fn release_shop_environment_cpu_sources_after_gpu_upload() {
    let mut g = SHOP_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if let ShopGlbCache::Ready(Some(cpu)) = &mut *g {
        for env in &mut cpu.environment_primitives {
            release_loaded_primitive_gpu_source_buffers(&mut env.mesh);
        }
        cpu.environment_primitives.shrink_to_fit();
    }
}

/// Default height multiplier for [`shop_env_world_scale`] when no debug override is active.
pub const SHOP_ENV_HEIGHT_SCALE: f32 = 1.0;

/// Multiplies glTF punctual **intensity** before upload (document-space inverse-square; see
/// `decal_atlas_uv.y` / `SsrGlobals.shop_punctual`). Default `1` uses authored intensities.
pub const SHOP_GLTF_LIGHT_INTENSITY_SCALE: f32 = 0.4;

/// Linear HDR gain for **shop** only: `2^-9` ≈ Don McCurdy glTF viewer exposure **−9** (EV on linear HDR).
/// Multiplied with [`ShopEnvLightingTune::linear_exposure`] and written to `shop_glb` / shop `lit_mesh` path.
pub const SHOP_ENV_LINEAR_EXPOSURE_BASE: f32 = 1.0 / 512.0; // 2^-9

/// Default **tuning** multiplier for linear HDR (debug overlay); shop applies [`SHOP_ENV_LINEAR_EXPOSURE_BASE`]
/// on top. Table scenes use this value alone (no shop base).
pub const SHOP_ENV_LINEAR_EXPOSURE: f32 = 1.0;

/// Linear HDR multiplier for tile-pack celebration (`Scene::TilePackCelebration`): no
/// [`crate::render::draw_cmd::DrawCmd::ShopEnvironment`], but showcase tiles still use shop-style
/// punctual lights — without this, `tile_hdr_tonemap` falls back to `linear_hdr ≈ 1` and faces clip.
/// Between full shop (`×`[`SHOP_ENV_LINEAR_EXPOSURE_BASE`]) and isolation showcase.
pub const TILE_PACK_CELEBRATION_HDR_LINEAR_EXPOSURE: f32 = 1.0 / 40.0;

/// Hemispheric fill in `shop_glb.wgsl` (`decal_atlas_uv.x`).
pub const SHOP_ENV_AMBIENT_SCALE: f32 = 0.0;

/// Applied in `lit_mesh.wgsl` as `shop_gltf_point_lights.extras.w` when
/// [`crate::render::draw_cmd::UiFrame::shop_env_gltf_punctual`] is set (`shop_glb.wgsl` ignores it).
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
        candle_light_color_mul: SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL,
    };
}

/// [`KHR_lights_punctual`] point light — positions in **document units** (same as mesh).
#[derive(Clone, Copy, Debug)]
pub struct ShopGlbEmbeddedPointLight {
    pub pos_doc: Vec3,
    /// Linear RGB from glTF before candle tint (see [`ShopGlbEmbeddedPointLight::is_candle`]).
    pub color_linear: [f32; 3],
    pub is_candle: bool,
    pub intensity: f32,
    pub range_doc: Option<f32>,
}

/// [`KHR_lights_punctual`] spot light — cone aims along node **−Z** in document space.
#[derive(Clone, Copy, Debug)]
pub struct ShopGlbEmbeddedSpotLight {
    pub pos_doc: Vec3,
    /// Unit vector from light toward illuminated surfaces (world/doc −Z axis).
    pub dir_doc: Vec3,
    pub color_linear: [f32; 3],
    pub is_candle: bool,
    pub intensity: f32,
    pub range_doc: Option<f32>,
    pub inner_cone_rad: f32,
    pub outer_cone_rad: f32,
}

#[inline]
pub fn shop_env_world_scale(window_h: f32, height_scale: f32) -> f32 {
    window_h.max(1e-6) * height_scale
}

/// One environment mesh primitive plus embedded glTF sampler parameters for GPU samplers.
pub struct ShopEnvPrimitiveCpu {
    pub mesh: LoadedPrimitive,
}

/// CPU triangle soup for one named GLB node (typically invisible anchor geometry). Vertices are
/// in the same pre-GPU-scale space as uploaded shop environment meshes (node transform applied).
#[derive(Clone)]
pub struct ShopCollisionMesh {
    pub node_name: String,
    pub triangles: Vec<[Vec3; 3]>,
}

/// Axis-aligned bounds of all decoded shop environment vertices (glTF document space).
#[derive(Clone, Copy, Debug)]
pub struct ShopEnvironmentBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl ShopEnvironmentBounds {
    #[inline]
    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn corners(self) -> [Vec3; 8] {
        let mn = self.min;
        let mx = self.max;
        [
            Vec3::new(mn.x, mn.y, mn.z),
            Vec3::new(mx.x, mn.y, mn.z),
            Vec3::new(mn.x, mx.y, mn.z),
            Vec3::new(mx.x, mx.y, mn.z),
            Vec3::new(mn.x, mn.y, mx.z),
            Vec3::new(mx.x, mn.y, mx.z),
            Vec3::new(mn.x, mx.y, mx.z),
            Vec3::new(mx.x, mx.y, mx.z),
        ]
    }
}

fn compute_environment_bounds(prims: &[ShopEnvPrimitiveCpu]) -> Option<ShopEnvironmentBounds> {
    let mut min_v = Vec3::splat(f32::INFINITY);
    let mut max_v = Vec3::splat(f32::NEG_INFINITY);
    for p in prims {
        for vtx in &p.mesh.vertices {
            let pos = Vec3::from(vtx.position);
            min_v = min_v.min(pos);
            max_v = max_v.max(pos);
        }
    }
    if !min_v.x.is_finite() || !max_v.x.is_finite() {
        return None;
    }
    Some(ShopEnvironmentBounds {
        min: min_v,
        max: max_v,
    })
}

/// `translate(-center_doc * s) * uniformScale(s)` — matches centered shop mesh + picking.
#[inline]
pub fn shop_env_model_matrix(window_h: f32, height_scale: f32, center_doc: Vec3) -> Mat4 {
    let s = shop_env_world_scale(window_h, height_scale);
    Mat4::from_translation(-center_doc * s) * Mat4::from_scale(Vec3::splat(s))
}

/// Model matrix using bounds center from the loaded Shop.glb, or plain scale if missing.
#[inline]
pub fn shop_env_model_matrix_from_cpu(
    window_h: f32,
    height_scale: f32,
    cpu: &ShopGlbCpu,
) -> Mat4 {
    let c = cpu
        .environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(Vec3::ZERO);
    shop_env_model_matrix(window_h, height_scale, c)
}

/// World-space AABB corners after centering and scale (for FOV fitting).
pub fn shop_world_bounds_corners_centered(
    window_h: f32,
    env_height_scale: f32,
    cpu: &ShopGlbCpu,
) -> Vec<Vec3> {
    let Some(bounds) = cpu.environment_bounds_doc else {
        return Vec::new();
    };
    let s = shop_env_world_scale(window_h, env_height_scale);
    let c = bounds.center();
    bounds
        .corners()
        .iter()
        .map(|p| (*p - c) * s)
        .collect()
}

/// Widen vertical FOV (only upward) so corners **in front of** the camera project inside `±margin_ndc`.
/// Corners behind the eye (common when the camera sits inside a tight AABB) are ignored so we don't
/// force a useless 170° search.
pub fn shop_camera_fit_fovy_for_corners(
    window_w: f32,
    window_h: f32,
    mut cam: CameraParams,
    corners_world: &[Vec3],
    margin_ndc: f32,
) -> CameraParams {
    if corners_world.is_empty() {
        return cam;
    }
    let eye = Vec3::from_array(cam.eye);
    let target = Vec3::from_array(cam.target);
    let forward = (target - eye).normalize_or_zero();
    if forward.length_squared() < 1e-12 {
        return cam;
    }
    let test_pts: Vec<Vec3> = corners_world
        .iter()
        .copied()
        .filter(|p| (*p - eye).dot(forward) > 0.25)
        .collect();
    if test_pts.len() < 4 {
        return cam;
    }

    let h = window_h.max(1e-6);
    let aspect = window_w / h;
    let far_p = h * 12.0;

    let projects_ok = |fovy_deg: f32| -> bool {
        let fov_y = fovy_deg.to_radians();
        let up = Vec3::from_array(cam.up);
        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(fov_y, aspect, 1.0, far_p);
        let vp = proj * view;
        for p in &test_pts {
            let clip = vp * Vec4::new(p.x, p.y, p.z, 1.0);
            if clip.w <= 0.01 {
                return false;
            }
            let inv_w = 1.0 / clip.w;
            let nx = clip.x * inv_w;
            let ny = clip.y * inv_w;
            if nx.abs() > margin_ndc || ny.abs() > margin_ndc {
                return false;
            }
        }
        true
    };

    if projects_ok(cam.fovy_deg) {
        return cam;
    }

    let mut lo = cam.fovy_deg;
    let mut hi = 170.0_f32;
    if !projects_ok(hi) {
        return cam;
    }

    for _ in 0..24 {
        if hi - lo < 0.05 {
            break;
        }
        let mid = (lo + hi) * 0.5;
        if projects_ok(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    cam.fovy_deg = hi;
    cam
}

/// Perspective camera baked into `Shop.glb` (positions in **document units**, same as mesh verts).
#[derive(Clone, Copy, Debug)]
pub struct ShopGlbEmbeddedCamera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fovy_deg: f32,
}

impl ShopGlbEmbeddedCamera {
    pub fn to_camera_params(
        &self,
        window_h: f32,
        env_height_scale: f32,
        center_doc: Vec3,
    ) -> CameraParams {
        let s = shop_env_world_scale(window_h, env_height_scale);
        let up = self.up.normalize_or_zero();
        let up = if up.length_squared() > 1e-12 {
            up
        } else {
            Vec3::Z
        };
        CameraParams {
            eye: ((self.eye - center_doc) * s).to_array(),
            target: ((self.target - center_doc) * s).to_array(),
            up: up.to_array(),
            fovy_deg: self.fovy_deg,
        }
    }
}

#[derive(Default)]
struct EmbeddedCameraHarvest {
    named: Option<ShopGlbEmbeddedCamera>,
    fallback: Option<ShopGlbEmbeddedCamera>,
}

impl EmbeddedCameraHarvest {
    fn pick(self) -> Option<ShopGlbEmbeddedCamera> {
        self.named.or(self.fallback)
    }

    fn insert(&mut self, name: &str, cam: ShopGlbEmbeddedCamera) {
        let key = name.to_ascii_lowercase();
        let preferred = matches!(key.as_str(), "camera" | "shopcamera" | "shop_camera");
        if preferred {
            if self.named.replace(cam).is_some() {
                log::warn!("Shop.glb: multiple preferred camera node names — using last");
            }
        } else if self.fallback.is_none() {
            self.fallback = Some(cam);
        }
    }
}

pub struct ShopGlbCpu {
    pub markers: HashMap<String, Mat4>,
    pub environment_primitives: Vec<ShopEnvPrimitiveCpu>,
    /// Bounds of environment mesh vertices (document space); drives centering + FOV fit.
    pub environment_bounds_doc: Option<ShopEnvironmentBounds>,
    /// Trimesh colliders for skipped-draw marker meshes (`shop_spawn_*`, `shop_player_*`).
    pub collision_meshes: Vec<ShopCollisionMesh>,
    /// First eligible perspective camera from the default scene, if any.
    pub embedded_perspective_camera: Option<ShopGlbEmbeddedCamera>,
    pub embedded_point_lights: Vec<ShopGlbEmbeddedPointLight>,
    pub embedded_spot_lights: Vec<ShopGlbEmbeddedSpotLight>,
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
pub const PLAYER_GOLD_DISH_MARKER: &str = "PlayerGoldDish";

fn is_marker_name(name: &str) -> bool {
    matches!(
        name,
        "exit_btn" | "restock_btn" | "journal_btn" | "Dish" | PLAYER_GOLD_DISH_MARKER
    )
        || name.starts_with("shop_spawn_relic_")
        || name.starts_with("shop_player_relic_")
        || name.starts_with("shop_player_consumable_")
}

/// Environment draw skip: anchor nodes often have collision/helper meshes that should not render.
/// Button markers are excluded — their mesh is a visible control and may bind focus UI.
fn skip_shop_env_mesh_for_node_name(name: &str) -> bool {
    name.starts_with("shop_spawn_relic_")
        || name.starts_with("shop_player_relic_")
        || name.starts_with("shop_player_consumable_")
}

fn decode_collision_triangles(
    primitive: gltf::Primitive<'_>,
    node_world: Mat4,
    buffers: &[Vec<u8>],
) -> anyhow::Result<Vec<[Vec3; 3]>> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .context("collision primitive has no POSITION")?
        .collect();
    let indices: Vec<u32> = if let Some(ids) = reader.read_indices() {
        ids.into_u32().collect()
    } else {
        (0..positions.len() as u32).collect()
    };
    let mut out = Vec::with_capacity(indices.len() / 3);
    for tri in indices.chunks_exact(3) {
        let p0 = node_world.transform_point3(Vec3::from(positions[tri[0] as usize]));
        let p1 = node_world.transform_point3(Vec3::from(positions[tri[1] as usize]));
        let p2 = node_world.transform_point3(Vec3::from(positions[tri[2] as usize]));
        out.push([p0, p1, p2]);
    }
    Ok(out)
}

fn shop_embedded_camera_from_node(world: Mat4, cam: gltf::Camera<'_>) -> Option<ShopGlbEmbeddedCamera> {
    let gltf::camera::Projection::Perspective(p) = cam.projection() else {
        return None;
    };
    let fovy_deg = p.yfov().to_degrees();
    let z_axis = world.z_axis.truncate();
    let y_axis = world.y_axis.truncate();
    let eye = world.w_axis.truncate();
    let z_len = z_axis.length();
    let y_len = y_axis.length();
    if !(z_len > 1e-20 && y_len > 1e-20) {
        return None;
    }
    let forward = (-z_axis / z_len).normalize();
    let up = (y_axis / y_len).normalize();
    let target = eye + forward;
    Some(ShopGlbEmbeddedCamera {
        eye,
        target,
        up,
        fovy_deg,
    })
}

/// Bake glTF `normalTexture.scale` into linear-ish RGBA normal texels.
fn apply_normal_scale_rgba8(pixels: &mut [u8], scale: f32) {
    if (scale - 1.0).abs() < 1e-5 {
        return;
    }
    for chunk in pixels.chunks_exact_mut(4) {
        let x = chunk[0] as f32 / 255.0 * 2.0 - 1.0;
        let y = chunk[1] as f32 / 255.0 * 2.0 - 1.0;
        let z = chunk[2] as f32 / 255.0 * 2.0 - 1.0;
        let nx = x * scale;
        let ny = y * scale;
        let nz = z;
        let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-8);
        let nx = (nx / len) * 0.5 + 0.5;
        let ny = (ny / len) * 0.5 + 0.5;
        let nz = (nz / len) * 0.5 + 0.5;
        chunk[0] = (nx.clamp(0.0, 1.0) * 255.0).round() as u8;
        chunk[1] = (ny.clamp(0.0, 1.0) * 255.0).round() as u8;
        chunk[2] = (nz.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

fn decode_primitive(
    primitive: gltf::Primitive<'_>,
    node_world: Mat4,
    buffers: &[Vec<u8>],
    images: &[gltf::image::Data],
) -> anyhow::Result<ShopEnvPrimitiveCpu> {
    let normal_xform = node_world.inverse().transpose();
    let material = primitive.material();
    let sampler_cpu = sampler_cpu_from_material(&material);

    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    let positions_local: Vec<[f32; 3]> = reader
        .read_positions()
        .context("primitive has no POSITION attribute")?
        .collect();

    let normals_local: Vec<[f32; 3]> = if let Some(n) = reader.read_normals() {
        n.collect()
    } else {
        vec![[0.0, 1.0, 0.0]; positions_local.len()]
    };

    anyhow::ensure!(
        normals_local.len() == positions_local.len(),
        "NORMAL count does not match POSITION count"
    );

    let pbr = material.pbr_metallic_roughness();
    let base_tex_coord = pbr.base_color_texture().map(|t| t.tex_coord()).unwrap_or(0);

    let mut uvs: Vec<[f32; 2]> = if let Some(tc) = reader.read_tex_coords(base_tex_coord) {
        tc.into_f32().collect()
    } else {
        vec![[0.0, 0.0]; positions_local.len()]
    };

    anyhow::ensure!(
        uvs.len() == positions_local.len(),
        "TEXCOORD count does not match POSITION count"
    );

    if let Some(tex_info) = pbr.base_color_texture() {
        apply_texture_transform(&mut uvs, &tex_info);
    }

    let mut uv_emr = uvs.clone();
    let mut tangents_local: Vec<[f32; 4]> = if let Some(t_iter) = reader.read_tangents() {
        let t: Vec<[f32; 4]> = t_iter.map(|a| [a[0], a[1], a[2], a[3]]).collect();
        anyhow::ensure!(
            t.len() == positions_local.len(),
            "TANGENT count does not match POSITION count"
        );
        t
    } else {
        Vec::new()
    };

    if let Some(nt) = material.normal_texture() {
        let set = nt.tex_coord();
        if set != base_tex_coord {
            uv_emr = if let Some(tc) = reader.read_tex_coords(set) {
                tc.into_f32().collect()
            } else {
                uvs.clone()
            };
            anyhow::ensure!(
                uv_emr.len() == positions_local.len(),
                "normal TEXCOORD count does not match POSITION count"
            );
            tangents_local.clear();
        }
    }

    let indices: Vec<u32> = if let Some(ids) = reader.read_indices() {
        ids.into_u32().collect()
    } else {
        (0..positions_local.len() as u32).collect()
    };

    if tangents_local.is_empty() {
        tangents_local =
            compute_vertex_tangents(&positions_local, &normals_local, &uv_emr, &indices);
    }

    let colors: Vec<[f32; 4]> = if let Some(iter) = reader.read_colors(0) {
        iter.into_rgba_f32().collect()
    } else {
        Vec::new()
    };

    let vertices: Vec<Vertex3dTex> = (0..positions_local.len())
        .map(|i| {
            let p = node_world.transform_point3(Vec3::from(positions_local[i]));
            let n = normal_xform
                .transform_vector3(Vec3::from(normals_local[i]))
                .normalize_or_zero();
            let tl = tangents_local[i];
            let t_loc = Vec3::new(tl[0], tl[1], tl[2]);
            let w = tl[3];
            let t_w = node_world.transform_vector3(t_loc).normalize_or_zero();
            let color = colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0, 1.0]);
            Vertex3dTex {
                position: p.into(),
                normal: n.into(),
                uv: uvs[i],
                tangent: [t_w.x, t_w.y, t_w.z, w],
                uv_emr: uv_emr[i],
                color,
            }
        })
        .collect();
    let factor = pbr.base_color_factor();

    let mut albedo_rgba = pbr.base_color_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        images.get(img_index).and_then(gltf_image_to_rgba8)
    });

    if albedo_rgba.is_none() && pbr.base_color_texture().is_some() {
        log::warn!(
            "Shop.glb primitive {}: base color texture present but image decode failed",
            primitive.index()
        );
    }

    match &mut albedo_rgba {
        Some((pix, _, _)) => multiply_rgba8_by_factor(pix, &factor),
        None => {
            let want_fallback_tex =
                factor != [1.0, 1.0, 1.0, 1.0] || pbr.base_color_texture().is_some();
            if want_fallback_tex {
                albedo_rgba = Some(solid_albedo_rgba8(&factor));
            }
        }
    }

    let normal_rgba = material.normal_texture().and_then(|nt| {
        let scale = nt.scale();
        let img_index = nt.texture().source().index();
        images
            .get(img_index)
            .and_then(gltf_image_to_rgba8)
            .map(|mut tex| {
                apply_normal_scale_rgba8(&mut tex.0, scale);
                tex
            })
    });

    if normal_rgba.is_none() && material.normal_texture().is_some() {
        log::warn!(
            "Shop.glb primitive {}: normal texture present but image decode failed",
            primitive.index()
        );
    }

    let metallic_roughness_rgba = pbr.metallic_roughness_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        images.get(img_index).and_then(gltf_image_to_rgba8)
    });

    let emissive_rgba = material.emissive_texture().and_then(|tex_info| {
        let img_index = tex_info.texture().source().index();
        images.get(img_index).and_then(gltf_image_to_rgba8)
    });

    let alpha_mode = GltfAlphaMode::from(material.alpha_mode());
    let alpha_cutoff = material.alpha_cutoff().unwrap_or(0.5);

    Ok(ShopEnvPrimitiveCpu {
        mesh: LoadedPrimitive {
            vertices,
            indices,
            albedo_rgba,
            normal_rgba,
            metallic_roughness_rgba,
            emissive_rgba,
            metallic_factor: pbr.metallic_factor(),
            roughness_factor: pbr.roughness_factor(),
            emissive_factor: material.emissive_factor(),
            alpha_mode,
            alpha_cutoff,
            double_sided: material.double_sided(),
            sampler: sampler_cpu,
        },
    })
}

fn harvest_khr_light(
    world: Mat4,
    light: gltf::khr_lights_punctual::Light<'_>,
    node_name: &str,
    points: &mut Vec<ShopGlbEmbeddedPointLight>,
    spots: &mut Vec<ShopGlbEmbeddedSpotLight>,
) {
    use gltf::khr_lights_punctual::Kind;

    let color_linear = light.color();
    let is_candle = node_name.starts_with(SHOP_GLTF_CANDLE_LIGHT_NODE_PREFIX);
    let intensity = light.intensity();
    let range_doc = light.range();

    match light.kind() {
        Kind::Point => {
            let pos_doc = world.transform_point3(Vec3::ZERO);
            points.push(ShopGlbEmbeddedPointLight {
                pos_doc,
                color_linear,
                is_candle,
                intensity,
                range_doc,
            });
        }
        Kind::Spot {
            inner_cone_angle,
            outer_cone_angle,
        } => {
            let z_axis = world.z_axis.truncate();
            let z_len = z_axis.length();
            if z_len < 1e-20 {
                log::warn!(
                    "Shop.glb: spot light {:?} has degenerate orientation — skipping",
                    node_name
                );
                return;
            }
            let dir_doc = (-z_axis / z_len).normalize();
            let pos_doc = world.transform_point3(Vec3::ZERO);
            let outer_rad = outer_cone_angle.max(1e-4);
            let inner_rad = inner_cone_angle.min(outer_rad).max(0.0);
            spots.push(ShopGlbEmbeddedSpotLight {
                pos_doc,
                dir_doc,
                color_linear,
                is_candle,
                intensity,
                range_doc,
                inner_cone_rad: inner_rad,
                outer_cone_rad: outer_rad,
            });
        }
        Kind::Directional => {
            log::debug!(
                "Shop.glb: skipping directional light on node {:?}",
                node_name
            );
        }
    }
}

fn walk_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    markers: &mut HashMap<String, Mat4>,
    env_primitives: &mut Vec<ShopEnvPrimitiveCpu>,
    collision_meshes: &mut Vec<ShopCollisionMesh>,
    embedded_cameras: &mut EmbeddedCameraHarvest,
    embedded_point_lights: &mut Vec<ShopGlbEmbeddedPointLight>,
    embedded_spot_lights: &mut Vec<ShopGlbEmbeddedSpotLight>,
    buffers: &[Vec<u8>],
    images: &[gltf::image::Data],
) -> anyhow::Result<()> {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent * local;
    let name = node.name().unwrap_or("");

    if let Some(light) = node.light() {
        harvest_khr_light(
            world,
            light,
            name,
            embedded_point_lights,
            embedded_spot_lights,
        );
    }

    if let Some(cam) = node.camera() {
        match cam.projection() {
            gltf::camera::Projection::Perspective(_) => {
                if let Some(ec) = shop_embedded_camera_from_node(world, cam) {
                    embedded_cameras.insert(name, ec);
                }
            }
            gltf::camera::Projection::Orthographic(_) => {
                log::debug!(
                    "Shop.glb: skipping orthographic camera on node {:?}",
                    name
                );
            }
        }
    }

    if is_marker_name(name) {
        if markers.insert(name.to_string(), world).is_some() {
            log::warn!(
                "Shop.glb: duplicate marker node name {:?} — using last transform",
                name
            );
        }
    }

    if let Some(mesh) = node.mesh() {
        if skip_shop_env_mesh_for_node_name(name) {
            if is_marker_name(name) {
                let mut tris = Vec::new();
                for prim in mesh.primitives() {
                    match decode_collision_triangles(prim, world, buffers) {
                        Ok(chunk) => tris.extend(chunk),
                        Err(e) => log::warn!("Shop.glb node {:?} collision: {e:#}", name),
                    }
                }
                if !tris.is_empty() {
                    collision_meshes.push(ShopCollisionMesh {
                        node_name: name.to_string(),
                        triangles: tris,
                    });
                }
            }
        } else {
            for prim in mesh.primitives() {
                env_primitives.push(decode_primitive(prim, world, buffers, images)?);
            }
        }
    }

    for child in node.children() {
        walk_node(
            child,
            world,
            markers,
            env_primitives,
            collision_meshes,
            embedded_cameras,
            embedded_point_lights,
            embedded_spot_lights,
            buffers,
            images,
        )?;
    }
    Ok(())
}

pub fn load_shop_glb_from_bytes(data: &[u8]) -> anyhow::Result<ShopGlbCpu> {
    let (document, buffers_vec, images) =
        gltf::import_slice(data).context("gltf::import_slice(Shop.glb)")?;

    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .context("Shop.glb has no scenes")?;

    let buffers: Vec<Vec<u8>> = buffers_vec.into_iter().map(|b| b.0).collect();

    let mut markers = HashMap::new();
    let mut environment_primitives = Vec::new();
    let mut collision_meshes = Vec::new();
    let mut embedded_cameras = EmbeddedCameraHarvest::default();
    let mut embedded_point_lights = Vec::new();
    let mut embedded_spot_lights = Vec::new();

    for node in scene.nodes() {
        walk_node(
            node,
            Mat4::IDENTITY,
            &mut markers,
            &mut environment_primitives,
            &mut collision_meshes,
            &mut embedded_cameras,
            &mut embedded_point_lights,
            &mut embedded_spot_lights,
            &buffers,
            &images,
        )?;
    }

    let embedded_perspective_camera = embedded_cameras.pick();
    let environment_bounds_doc = compute_environment_bounds(&environment_primitives);

    Ok(ShopGlbCpu {
        markers,
        environment_primitives,
        environment_bounds_doc,
        collision_meshes,
        embedded_perspective_camera,
        embedded_point_lights,
        embedded_spot_lights,
    })
}

/// Shop camera from embedded GLB perspective camera, scaled like marker geometry.
#[inline]
pub fn shop_camera_from_glb_if_present(window_h: f32, env_height_scale: f32) -> Option<CameraParams> {
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
pub fn marker_translation(cpu: &ShopGlbCpu, name: &str) -> Option<Vec3> {
    let center_doc = cpu
        .environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(Vec3::ZERO);
    cpu.markers
        .get(name)
        .map(|m| m.transform_point3(Vec3::ZERO) - center_doc)
}
