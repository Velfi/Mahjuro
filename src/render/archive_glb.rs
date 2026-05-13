//! [`archive.glb`](../../../assets/archive.glb) — Archive (`CollectionScene`) room.
//!
//! ## Node names (Blender object names → glTF nodes)
//!
//! - `sign_description_left` / `sign_description_right` — description boards; runtime draws one
//!   mesh (see `archive_env_skip_description_prim`) based on cursor so the active board is away
//!   from the pointer; catalog copy is CPU-rasterized into a shared decal atlas and composited in
//!   `shop_glb.wgsl` on those meshes (vertex `COLOR_0.a = 2` tag from `decode_env_primitive`).
//! - `archive_spawn_item.001` … `archive_spawn_item.021` — 21 item anchors (3×7 window into the tab catalogue).
//! - `section_buttons_left_bound` / `section_buttons_right_bound` — bounds volumes for tab plaques.
//! - `archive_spawn_focused_item` — large featured / inspect anchor.
//!
//! **Description copy:** name/body text is **CPU-rasterized** in [`CollectionScene`](../../scenes/collection.rs)
//! into the archive decal texture (see `sync_archive_description_decal_texture`).
//! [`archive_description_sign_use_left_for_cursor`] picks the side;
//! [`UiFrame::archive_description_sign_use_left`] culls the opposite GLB primitive so only one
//! board draws the active copy.
//!
//! Export **without Draco** (`KHR_draco_mesh_compression`). Use Blender glTF **Lighting Mode → Standard**
//! when using `KHR_lights_punctual`.
//!
//! Decodes through [`crate::render::room_env_gltf`]; GPU path matches shop/hallway (`shop_glb.wgsl`).

use std::sync::RwLock;

use glam::{Mat4, Vec3};

use crate::render::draw_cmd::CameraParams;
use crate::render::room_env_gltf::{
    glb_punctual_range_world_upload, RoomEnvWalkHooks, RoomMeshPolicy,
};
use crate::render::shop_glb::{self, load_room_glb_from_bytes, RoomGlbCpu, ShopEnvLightingTune};
use crate::render::wgpu_renderer::{PointLight, SpotLight, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS};
use crate::render::world_space::surface_anchor_from_world_xyz;

pub const SIGN_DESCRIPTION_LEFT: &str = "sign_description_left";
pub const SIGN_DESCRIPTION_RIGHT: &str = "sign_description_right";
pub const SECTION_BUTTONS_LEFT_BOUND: &str = "section_buttons_left_bound";
pub const SECTION_BUTTONS_RIGHT_BOUND: &str = "section_buttons_right_bound";
pub const ARCHIVE_SPAWN_FOCUSED_ITEM: &str = "archive_spawn_focused_item";

/// Virtual host extents for [`crate::render::decal::decal_dimensions`] when rasterizing description
/// copy onto `sign_description_*` meshes (must match init in `build.rs`).
pub const ARCHIVE_DESCRIPTION_DECAL_HOST_EXTENTS: [f32; 3] = [4.0, 1.0, 1.0];

/// Linear HDR exposure multiplier for `archive.glb` (applied when embedded punctual lights are active).
pub const ARCHIVE_ENV_LINEAR_EXPOSURE_MUL: f32 = 1.85;

/// Minimum hemispheric ambient (`decal_atlas_uv.x`) for this room.
pub const ARCHIVE_ENV_AMBIENT_SCALE_MIN: f32 = 0.075;

/// Visible item slots in the authored room (3×7 = 21).
pub const ARCHIVE_SLOT_COLS: usize = 3;
pub const ARCHIVE_SLOT_ROWS: usize = 7;
pub const ARCHIVE_SLOT_COUNT: usize = ARCHIVE_SLOT_COLS * ARCHIVE_SLOT_ROWS;

/// glTF object names `archive_spawn_item.001` … `archive_spawn_item.021` (stable keys, no heap allocs).
pub const ARCHIVE_SPAWN_ITEM_NODE_NAMES: [&str; ARCHIVE_SLOT_COUNT] = [
    "archive_spawn_item.001",
    "archive_spawn_item.002",
    "archive_spawn_item.003",
    "archive_spawn_item.004",
    "archive_spawn_item.005",
    "archive_spawn_item.006",
    "archive_spawn_item.007",
    "archive_spawn_item.008",
    "archive_spawn_item.009",
    "archive_spawn_item.010",
    "archive_spawn_item.011",
    "archive_spawn_item.012",
    "archive_spawn_item.013",
    "archive_spawn_item.014",
    "archive_spawn_item.015",
    "archive_spawn_item.016",
    "archive_spawn_item.017",
    "archive_spawn_item.018",
    "archive_spawn_item.019",
    "archive_spawn_item.020",
    "archive_spawn_item.021",
];

#[inline]
pub fn archive_spawn_item_marker_name(slot: usize) -> &'static str {
    ARCHIVE_SPAWN_ITEM_NODE_NAMES
        .get(slot)
        .copied()
        .unwrap_or(ARCHIVE_SPAWN_ITEM_NODE_NAMES[ARCHIVE_SLOT_COUNT - 1])
}

enum ArchiveGlbCache {
    Uninit,
    Ready(Option<RoomGlbCpu>),
}

static ARCHIVE_GLB_CPU: RwLock<ArchiveGlbCache> = RwLock::new(ArchiveGlbCache::Uninit);

fn ensure_archive_glb_loaded() {
    let mut w = ARCHIVE_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if !matches!(*w, ArchiveGlbCache::Uninit) {
        return;
    }
    let ready = if let Some(file) = crate::asset_path::get("archive.glb") {
        match load_archive_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::debug!(
                    "archive.glb: {} marker(s), {} draw primitive(s)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                );
                Some(cpu)
            }
            Err(e) => {
                log::error!("archive.glb failed to load: {e:#}");
                None
            }
        }
    } else {
        log::debug!("archive.glb not embedded — Archive uses procedural layout");
        None
    };
    *w = ArchiveGlbCache::Ready(ready);
}

/// `true` when `archive.glb` loaded and has drawable environment geometry.
pub fn archive_room_draw_ready() -> bool {
    with_archive_glb_cpu(|opt| opt.is_some_and(|c| !c.environment_primitives.is_empty()))
}

pub fn with_archive_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_archive_glb_loaded();
    let g = ARCHIVE_GLB_CPU.read().unwrap_or_else(|e| e.into_inner());
    match &*g {
        ArchiveGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        ArchiveGlbCache::Ready(None) => f(None),
        ArchiveGlbCache::Uninit => {
            log::warn!("archive.glb cache still Uninit after ensure — treating as absent");
            f(None)
        }
    }
}

pub fn release_archive_environment_cpu_sources_after_gpu_upload() {
    let mut g = ARCHIVE_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if let ArchiveGlbCache::Ready(Some(cpu)) = &mut *g {
        shop_glb::release_room_environment_primitives_cpu(cpu);
    }
}

#[inline]
fn is_archive_spawn_item_name(name: &str) -> bool {
    name.starts_with("archive_spawn_item.")
}

#[inline]
fn is_archive_marker_name(name: &str) -> bool {
    matches!(
        name,
        SIGN_DESCRIPTION_LEFT
            | SIGN_DESCRIPTION_RIGHT
            | SECTION_BUTTONS_LEFT_BOUND
            | SECTION_BUTTONS_RIGHT_BOUND
            | ARCHIVE_SPAWN_FOCUSED_ITEM
    ) || is_archive_spawn_item_name(name)
}

#[derive(Copy, Clone)]
struct ArchiveRoomWalkHooks;

impl RoomEnvWalkHooks for ArchiveRoomWalkHooks {
    fn is_marker(&self, name: &str) -> bool {
        is_archive_marker_name(name)
    }

    fn mesh_policy(&self, name: &str) -> RoomMeshPolicy {
        if matches!(name, SIGN_DESCRIPTION_LEFT | SIGN_DESCRIPTION_RIGHT) {
            RoomMeshPolicy::EnvironmentDraw
        } else if matches!(
            name,
            SECTION_BUTTONS_LEFT_BOUND | SECTION_BUTTONS_RIGHT_BOUND
        ) || is_archive_spawn_item_name(name)
            || name == ARCHIVE_SPAWN_FOCUSED_ITEM
        {
            RoomMeshPolicy::SkipDrawCollisionIfMarker
        } else {
            RoomMeshPolicy::EnvironmentDraw
        }
    }

    fn log_asset_label(&self) -> &'static str {
        "archive.glb"
    }
}

pub fn load_archive_glb_from_bytes(data: &[u8]) -> anyhow::Result<RoomGlbCpu> {
    let cpu = load_room_glb_from_bytes(
        data,
        "gltf::import_slice(archive.glb)",
        "archive.glb has no scenes",
        &ArchiveRoomWalkHooks,
    )?;
    // Keep collision soups for section bounds (screen-rect projection); spawn empties contribute none.
    Ok(cpu)
}

/// World-space marker translation (centered room basis), consistent with shop/hallway mesh.
#[allow(dead_code)] // Public helper for tooling / future UI; collection uses `archive_marker_world_mat4`.
pub fn archive_marker_world(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> Option<Vec3> {
    let t = shop_glb::marker_translation(cpu, name)?;
    let s = shop_glb::shop_env_world_scale(window_h, env_height_scale);
    Some(t * s)
}

/// Full transform of marker origin in **world** space (includes room model matrix).
pub fn archive_marker_world_mat4(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> Option<Mat4> {
    let m = shop_glb::shop_env_model_matrix_from_cpu(window_h, env_height_scale, cpu);
    let node = cpu.marker_node_transform_doc(name)?;
    Some(m * node)
}

pub fn archive_camera_from_glb_if_present(
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    with_archive_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        cpu.embedded_perspective_camera
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

pub fn archive_camera_base(w: f32, h: f32, env_h: f32) -> CameraParams {
    let from_glb = archive_camera_from_glb_if_present(h, env_h);
    let cam = from_glb.unwrap_or_else(|| CameraParams {
        eye: [0.0, -h * 1.15, h * 0.48],
        target: [0.0, h * 0.02, h * 0.12],
        up: [0.0, 0.0, 1.0],
        fovy_deg: 50.0,
    });
    if from_glb.is_some() {
        return cam;
    }
    with_archive_glb_cpu(|opt| {
        if let Some(cpu) = opt {
            let corners = shop_glb::shop_world_bounds_corners_centered(h, env_h, cpu);
            shop_glb::shop_camera_fit_fovy_for_corners(w, h, cam, &corners, 0.94)
        } else {
            cam
        }
    })
}

/// When both description signs have projected bounds, compares `cursor_x` to the midpoint between
/// their screen centers so the visible quad tends to sit **away** from the pointer. One-sided
/// assets lock to that side; if neither has bounds, returns [`None`] (caller may use window center).
pub fn archive_description_sign_use_left_for_cursor(
    win_w: f32,
    win_h: f32,
    env_h: f32,
    cam: &CameraParams,
    cursor_x: f32,
    cpu: &RoomGlbCpu,
) -> Option<bool> {
    let rl = shop_glb::screen_rect_for_marker_mesh_bounds(
        win_w,
        win_h,
        cam,
        env_h,
        cpu,
        SIGN_DESCRIPTION_LEFT,
        8.0,
        8.0,
    );
    let rr = shop_glb::screen_rect_for_marker_mesh_bounds(
        win_w,
        win_h,
        cam,
        env_h,
        cpu,
        SIGN_DESCRIPTION_RIGHT,
        8.0,
        8.0,
    );
    match (rl, rr) {
        (Some(l), Some(r)) => {
            let mid_l = l[0] + l[2] * 0.5;
            let mid_r = r[0] + r[2] * 0.5;
            let mid = (mid_l + mid_r) * 0.5;
            Some(cursor_x >= mid)
        }
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

pub fn archive_glb_has_embedded_lights() -> bool {
    with_archive_glb_cpu(|opt| {
        opt.is_some_and(|cpu| {
            !cpu.embedded_point_lights.is_empty() || !cpu.embedded_spot_lights.is_empty()
        })
    })
}

fn gltf_punctual_linear_rgb(
    raw: [f32; 3],
    is_candle: bool,
    tune: &ShopEnvLightingTune,
) -> [f32; 3] {
    if is_candle {
        [
            (raw[0] * tune.candle_light_color_mul[0]).clamp(0.0, 1.0),
            (raw[1] * tune.candle_light_color_mul[1]).clamp(0.0, 1.0),
            (raw[2] * tune.candle_light_color_mul[2]).clamp(0.0, 1.0),
        ]
    } else {
        raw
    }
}

pub fn archive_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &ShopEnvLightingTune,
) -> Vec<PointLight> {
    with_archive_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return Vec::new();
        };
        if cpu.embedded_point_lights.is_empty() {
            return Vec::new();
        }
        let s = shop_glb::shop_env_world_scale(h, env_h);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        let budget = MAX_POINT_LIGHTS.saturating_sub(2);
        if cpu.embedded_point_lights.len() > budget {
            log::warn!(
                "archive.glb: {} point lights exceed budget ({}) — truncating",
                cpu.embedded_point_lights.len(),
                budget
            );
        }
        cpu.embedded_point_lights
            .iter()
            .take(budget)
            .map(|l| {
                let world = (l.pos_doc - center_doc) * s;
                let radius = glb_punctual_range_world_upload(h, s, l.range_doc);
                PointLight {
                    pos: surface_anchor_from_world_xyz(w, h, world),
                    radius,
                    color: gltf_punctual_linear_rgb(l.color_linear, l.is_candle, tune),
                    intensity: (l.intensity * tune.gltf_light_intensity_scale).max(0.0),
                }
            })
            .collect()
    })
}

pub fn archive_embedded_spot_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &ShopEnvLightingTune,
) -> Vec<SpotLight> {
    if !archive_glb_has_embedded_lights() {
        return Vec::new();
    }
    with_archive_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return Vec::new();
        };
        if cpu.embedded_spot_lights.is_empty() {
            return Vec::new();
        }
        let s = shop_glb::shop_env_world_scale(h, env_h);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        if cpu.embedded_spot_lights.len() > MAX_SPOT_LIGHTS {
            log::warn!(
                "archive.glb: {} spot lights exceed {} — truncating",
                cpu.embedded_spot_lights.len(),
                MAX_SPOT_LIGHTS
            );
        }
        cpu.embedded_spot_lights
            .iter()
            .take(MAX_SPOT_LIGHTS)
            .filter_map(|l| {
                let dir_w = l.dir_doc.normalize_or_zero();
                if dir_w.length_squared() < 1e-12 {
                    return None;
                }
                let world = (l.pos_doc - center_doc) * s;
                let radius = glb_punctual_range_world_upload(h, s, l.range_doc);
                let cos_outer = l.outer_cone_rad.cos();
                let cos_inner = l.inner_cone_rad.cos().max(cos_outer);
                Some(SpotLight {
                    pos: surface_anchor_from_world_xyz(w, h, world),
                    dir: dir_w.to_array(),
                    radius,
                    cos_outer,
                    cos_inner,
                    color: gltf_punctual_linear_rgb(l.color_linear, l.is_candle, tune),
                    intensity: (l.intensity * tune.gltf_light_intensity_scale).max(0.0),
                })
            })
            .collect()
    })
}
