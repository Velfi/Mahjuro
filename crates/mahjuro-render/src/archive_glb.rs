//! [`archive.glb`](../../../assets/3d/archive.glb) — Archive (`ArchiveScene`) room.
//!
//! ## Node names (Blender object names → glTF nodes)
//!
//! - `sign_description_left` / `sign_description_right` — grid-mode description boards; runtime
//!   draws one mesh (see `archive_env_skip_description_prim`) based on cursor so the active board
//!   is away from the pointer; browse copy is CPU-rasterized into a shared decal atlas.
//! - `inspect_plaque` — inspect-mode description board beside the turntable; same decal atlas
//!   path (explicit room-env PBR feature flags from `room_gpu_load.rs`); hidden outside item inspect.
//! - `plaque_backing` — ornate frame behind [`INSPECT_PLAQUE`]; hidden outside item inspect.
//! - `archive_spawn_item.001` … `archive_spawn_item.021` — 21 item anchors (3×7 window into the tab catalogue).
//! - `btn_relics_tab`, `btn_zodiacs_tab`, `btn_bosses_tab`, `btn_talismans_tab`, `btn_chronicle_tab` —
//!   section tabs (meshes draw; hit rects project mesh AABBs like shop `exit_btn`).
//! - `btn_main_menu`, `btn_switch_save` — title-bar chrome.
//! - `btn_page_left`, `btn_page_right` — cabinet page step (hidden at first/last page).
//! - `archive_spawn_focused_item` — large featured / inspect anchor.
//!
//! **Description copy:** name/body text is **CPU-rasterized** in [`ArchiveScene`](../../src/scenes/archive.rs)
//! into the archive decal texture (see `sync_archive_description_decal_texture`).
//! [`archive_description_sign_use_left_for_ref_x`] picks the side (cursor X in
//! [`crate::ui::input::InputMode::Cursor`]; projected focused-item X in keyboard / controller mode);
//! [`UiFrame::archive_description_sign_use_left`] culls the opposite GLB primitive so only one
//! board draws the active copy.
//!
//! Export **without Draco** (`KHR_draco_mesh_compression`). Use Blender glTF **Lighting Mode → Standard**
//! when using `KHR_lights_punctual`.
//!
//! Decodes through [`crate::room_env_gltf`]; GPU path matches shop/hallway (`room_glb.wgsl`).

use parking_lot::RwLock;

use glam::{Mat4, Vec3};

use crate::draw_cmd::CameraParams;
use crate::room_env_gltf::{RoomEnvWalkHooks, RoomMeshPolicy};
use crate::room_glb::{self, RoomEnvLightingTune, RoomGlbCpu, load_room_glb_from_bytes};
use crate::wgpu_renderer::PointLight;

pub const SIGN_DESCRIPTION_LEFT: &str = "sign_description_left";
pub const SIGN_DESCRIPTION_RIGHT: &str = "sign_description_right";
pub const INSPECT_PLAQUE: &str = "inspect_plaque";
pub const PLAQUE_BACKING: &str = "plaque_backing";
pub const ARCHIVE_SPAWN_FOCUSED_ITEM: &str = "archive_spawn_focused_item";

pub const BTN_RELICS_TAB: &str = "btn_relics_tab";
pub const BTN_ZODIACS_TAB: &str = "btn_zodiacs_tab";
pub const BTN_BOSSES_TAB: &str = "btn_bosses_tab";
pub const BTN_TALISMANS_TAB: &str = "btn_talismans_tab";
pub const BTN_CHRONICLE_TAB: &str = "btn_chronicle_tab";
pub const BTN_MAIN_MENU: &str = "btn_main_menu";
pub const BTN_SWITCH_SAVE: &str = "btn_switch_save";
pub const BTN_PAGE_LEFT: &str = "btn_page_left";
pub const BTN_PAGE_RIGHT: &str = "btn_page_right";

/// Tab button glTF nodes in [`crate::scenes::archive::TABS`] order (Relics → Talismans → Yaku → Bosses → Chronicle).
pub const ARCHIVE_TAB_BUTTON_NODES: [&str; 5] = [
    BTN_RELICS_TAB,
    BTN_TALISMANS_TAB,
    BTN_ZODIACS_TAB,
    BTN_BOSSES_TAB,
    BTN_CHRONICLE_TAB,
];

/// Fallback host extents for [`crate::decal::decal_dimensions`] when the archive `.glb`
/// is missing. Live archive decal sizing reads the actual sign-face aspect via
/// [`archive_sign_description_decal_extents`].
pub const ARCHIVE_DESCRIPTION_DECAL_HOST_EXTENTS: [f32; 3] = [1.0, 1.0, 1.0];

fn archive_decal_host_extents_for_marker(cpu: &RoomGlbCpu, node: &str) -> Option<[f32; 3]> {
    let bounds = cpu.marker_mesh_bounds_doc_for(node).copied().or_else(|| {
        crate::room_env_gltf::room_env_primitive_bounds_doc(&cpu.environment_primitives, node)
    })?;
    let diag = bounds.max - bounds.min;
    let mut e = [diag.x.abs(), diag.y.abs(), diag.z.abs()];
    // Sort ascending so e[2] = longest face edge, e[1] = shorter face edge, e[0] = thickness.
    e.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let long = e[2].max(1e-6);
    let short = e[1].max(1e-6);
    // `decal_dimensions(Fit)` does `extents[0] / extents[1].max(1.0)` — normalize the short
    // edge to exactly 1.0 so doc-space units (which can be < 1) still yield the true aspect.
    Some([long / short, 1.0, 1.0])
}

/// Read the `sign_description_*` mesh face aspect from a loaded archive `.glb`.
pub fn archive_sign_description_decal_extents_for(cpu: &RoomGlbCpu) -> [f32; 3] {
    archive_decal_host_extents_for_marker(cpu, SIGN_DESCRIPTION_LEFT)
        .or_else(|| archive_decal_host_extents_for_marker(cpu, SIGN_DESCRIPTION_RIGHT))
        .unwrap_or(ARCHIVE_DESCRIPTION_DECAL_HOST_EXTENTS)
}

/// Read the `inspect_plaque` mesh face aspect for the inspect decal atlas.
///
/// When authored UV runs **u** along local **Y** (not **X** like the sign boards), decode applies
/// [`crate::room_env_gltf::archive_inspect_plaque_decal_uv`] (decal **U** along local **+Y**) and this
/// returns a **portrait** aspect (`extents[0] < 1`).
pub fn archive_inspect_plaque_decal_extents_for(cpu: &RoomGlbCpu) -> [f32; 3] {
    if let Some(prim) = cpu
        .environment_primitives
        .iter()
        .find(|p| p.gltf_node_name.as_deref() == Some(INSPECT_PLAQUE))
    {
        return [prim.archive_decal_face_aspect, 1.0, 1.0];
    }
    archive_decal_host_extents_for_marker(cpu, INSPECT_PLAQUE)
        .unwrap_or(ARCHIVE_DESCRIPTION_DECAL_HOST_EXTENTS)
}

/// Convenience wrapper that opens the archive `.glb` cache. **Do not call from inside an
/// existing [`with_archive_glb_cpu`] closure** — use [`archive_sign_description_decal_extents_for`]
/// directly instead.
pub fn archive_sign_description_decal_extents() -> [f32; 3] {
    with_archive_glb_cpu(|opt| {
        opt.map(archive_sign_description_decal_extents_for)
            .unwrap_or(ARCHIVE_DESCRIPTION_DECAL_HOST_EXTENTS)
    })
}

/// Convenience wrapper for [`archive_inspect_plaque_decal_extents_for`].
pub fn archive_inspect_plaque_decal_extents() -> [f32; 3] {
    with_archive_glb_cpu(|opt| {
        opt.map(archive_inspect_plaque_decal_extents_for)
            .unwrap_or(ARCHIVE_DESCRIPTION_DECAL_HOST_EXTENTS)
    })
}

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
    Ready(Option<Box<RoomGlbCpu>>),
}

static ARCHIVE_GLB_CPU: RwLock<ArchiveGlbCache> = RwLock::new(ArchiveGlbCache::Uninit);

/// True when `archive.glb` has been decoded into the process cache.
pub fn archive_cpu_decoded() -> bool {
    let g = ARCHIVE_GLB_CPU.read();
    matches!(&*g, ArchiveGlbCache::Ready(Some(_)))
}

/// True when decoded environment meshes are present and not yet released for GPU upload.
pub fn archive_cpu_ready_for_gpu_upload() -> bool {
    let g = ARCHIVE_GLB_CPU.read();
    match &*g {
        ArchiveGlbCache::Ready(Some(cpu)) => {
            !cpu.environment_primitives.is_empty() && !cpu.environment_primitives_released
        }
        _ => false,
    }
}

/// Decode `archive.glb` into the process-wide CPU cache (main or prefetch thread).
pub fn decode_archive_glb_into_cache() {
    let mut w = ARCHIVE_GLB_CPU.write();
    if matches!(&*w, ArchiveGlbCache::Ready(Some(cpu)) if !room_glb::room_glb_cpu_needs_environment_mesh_reload(cpu)
        && !room_glb::room_glb_cpu_stale_environment_for_gpu_upload(cpu))
    {
        return;
    }
    let ready = if let Some(file) = mahjuro_assets::asset_path::get("3d/archive.glb") {
        match load_archive_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::debug!(
                    "archive.glb: {} marker(s), {} draw primitive(s)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                );
                Some(cpu)
            }
            Err(e) => panic!("archive.glb failed to load: {e:#}"),
        }
    } else {
        panic!("archive.glb not embedded; required when loading archive room");
    };
    *w = ArchiveGlbCache::Ready(ready.map(Box::new));
}

fn ensure_archive_glb_loaded() {
    crate::room_preload::join_archive_cpu_prefetch_blocking();
    let mut w = ARCHIVE_GLB_CPU.write();
    match &*w {
        ArchiveGlbCache::Uninit => {}
        ArchiveGlbCache::Ready(Some(cpu))
            if room_glb::room_glb_cpu_needs_environment_mesh_reload(cpu)
                || room_glb::room_glb_cpu_stale_environment_for_gpu_upload(cpu) =>
        {
            *w = ArchiveGlbCache::Uninit;
        }
        _ => return,
    }
    drop(w);
    decode_archive_glb_into_cache();
}

pub fn with_archive_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_archive_glb_loaded();
    let g = ARCHIVE_GLB_CPU.read();
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
    let mut g = ARCHIVE_GLB_CPU.write();
    if let ArchiveGlbCache::Ready(Some(cpu)) = &mut *g {
        room_glb::release_room_environment_primitives_cpu(cpu);
    }
}

pub fn clear_archive_glb_cpu_cache() {
    *ARCHIVE_GLB_CPU.write() = ArchiveGlbCache::Uninit;
}

#[inline]
fn is_archive_spawn_item_name(name: &str) -> bool {
    name.starts_with("archive_spawn_item.")
}

#[inline]
fn is_archive_button_node(name: &str) -> bool {
    matches!(
        name,
        BTN_RELICS_TAB
            | BTN_ZODIACS_TAB
            | BTN_BOSSES_TAB
            | BTN_TALISMANS_TAB
            | BTN_CHRONICLE_TAB
            | BTN_MAIN_MENU
            | BTN_SWITCH_SAVE
            | BTN_PAGE_LEFT
            | BTN_PAGE_RIGHT
    )
}

#[inline]
fn is_archive_marker_name(name: &str) -> bool {
    is_archive_button_node(name)
        || matches!(
            name,
            SIGN_DESCRIPTION_LEFT
                | SIGN_DESCRIPTION_RIGHT
                | INSPECT_PLAQUE
                | ARCHIVE_SPAWN_FOCUSED_ITEM
        )
        || is_archive_spawn_item_name(name)
}

#[derive(Copy, Clone)]
struct ArchiveRoomWalkHooks;

impl RoomEnvWalkHooks for ArchiveRoomWalkHooks {
    fn is_marker(&self, name: &str) -> bool {
        is_archive_marker_name(name)
    }

    fn mesh_policy(&self, name: &str) -> RoomMeshPolicy {
        if matches!(
            name,
            SIGN_DESCRIPTION_LEFT | SIGN_DESCRIPTION_RIGHT | INSPECT_PLAQUE
        ) || is_archive_button_node(name)
        {
            RoomMeshPolicy::EnvironmentDraw
        } else if is_archive_spawn_item_name(name) || name == ARCHIVE_SPAWN_FOCUSED_ITEM {
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
    // Spawn anchors stay collision-only; tab/chrome buttons draw like shop controls.
    Ok(cpu)
}

/// Full transform of marker origin in **world** space (includes room model matrix).
pub fn archive_marker_world_mat4(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> Option<Mat4> {
    let m = room_glb::room_env_model_matrix_from_cpu(window_h, env_height_scale, cpu);
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
    with_archive_glb_cpu(|opt| {
        let mut cam = from_glb.unwrap_or_else(|| CameraParams {
            eye: [0.0, -h * 1.15, h * 0.48],
            target: [0.0, h * 0.02, h * 0.12],
            up: [0.0, 0.0, 1.0],
            projection: crate::draw_cmd::CameraProjection::Perspective { fovy_deg: 50.0 },
            clip_near: None,
            clip_far: None,
        });
        if from_glb.is_none()
            && let Some(cpu) = opt
        {
            let corners = room_glb::room_world_bounds_corners_centered(h, env_h, cpu);
            cam = room_glb::room_camera_fit_fovy_for_corners(w, h, cam, &corners, 0.94);
        }
        if let Some(cpu) = opt {
            cam = room_glb::room_camera_with_room_clip_planes(cam, h, env_h, cpu);
            // `room_camera_fit_clip_planes` sets `clip_near` near the room AABB entry along the
            // look ray, which can clip nearby title-bar `btn_*` meshes. Keep the tuned `clip_far`
            // for depth precision; use the default near from [`CameraParams::clip_planes`]
            // instead (`clip_near: None` → 1 world unit at this scale).
            cam.clip_near = None;
        }
        cam
    })
}

/// Screen AABB of the description board that is currently visible (the opposite side is culled).
pub fn archive_active_description_sign_screen_rect(
    win_w: f32,
    win_h: f32,
    env_h: f32,
    cam: &crate::draw_cmd::CameraParams,
    use_left: Option<bool>,
) -> Option<[f32; 4]> {
    let use_left = use_left?;
    with_archive_glb_cpu(|opt| {
        let cpu = opt?;
        let node = if use_left {
            SIGN_DESCRIPTION_LEFT
        } else {
            SIGN_DESCRIPTION_RIGHT
        };
        room_glb::screen_rect_for_marker_mesh_bounds(&room_glb::MarkerScreenRectParams {
            win_w,
            win_h,
            cam,
            env_height_scale: env_h,
            cpu,
            node_name: node,
            min_rw: 8.0,
            min_rh: 8.0,
        })
    })
}

/// When both description signs have projected bounds, compares `ref_x` (screen-X in window pixels)
/// to the midpoint between their screen centers so the visible quad tends to sit **away** from
/// `ref_x`. Callers pass the cursor X in [`crate::ui::input::InputMode::Cursor`] and the focused
/// item's projected X in keyboard / controller mode so the active sign always sits opposite the
/// thing the player is looking at. One-sided assets lock to that side; if neither has bounds,
/// returns [`None`] (caller may use window center).
pub fn archive_description_sign_use_left_for_ref_x(
    win_w: f32,
    win_h: f32,
    env_h: f32,
    cam: &CameraParams,
    ref_x: f32,
    cpu: &RoomGlbCpu,
) -> Option<bool> {
    let rl = room_glb::screen_rect_for_marker_mesh_bounds(&room_glb::MarkerScreenRectParams {
        win_w,
        win_h,
        cam,
        env_height_scale: env_h,
        cpu,
        node_name: SIGN_DESCRIPTION_LEFT,
        min_rw: 8.0,
        min_rh: 8.0,
    });
    let rr = room_glb::screen_rect_for_marker_mesh_bounds(&room_glb::MarkerScreenRectParams {
        win_w,
        win_h,
        cam,
        env_height_scale: env_h,
        cpu,
        node_name: SIGN_DESCRIPTION_RIGHT,
        min_rw: 8.0,
        min_rh: 8.0,
    });
    match (rl, rr) {
        (Some(l), Some(r)) => {
            let mid_l = l[0] + l[2] * 0.5;
            let mid_r = r[0] + r[2] * 0.5;
            let mid = (mid_l + mid_r) * 0.5;
            Some(ref_x >= mid)
        }
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

/// Project a marker's world position to screen-X in window pixels. Used by
/// [`archive_description_sign_use_left_for_ref_x`] callers in keyboard / controller mode to
/// reference the focused item rather than the cursor.
pub fn archive_marker_screen_x(
    win_w: f32,
    win_h: f32,
    env_h: f32,
    cam: &CameraParams,
    cpu: &RoomGlbCpu,
    name: &str,
) -> Option<f32> {
    let m = archive_marker_world_mat4(win_h, env_h, cpu, name)?;
    let p = m.transform_point3(Vec3::ZERO);
    let (sx, _) = cam.project_world_to_screen(win_w, win_h, p);
    Some(sx)
}

pub fn archive_glb_has_embedded_lights() -> bool {
    with_archive_glb_cpu(|opt| {
        opt.is_some_and(crate::room_gltf_punctual::room_glb_has_embedded_lights)
    })
}

pub fn archive_embedded_point_lights_runtime_tagged(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<crate::room_gltf_punctual::EmbeddedPointLightRuntime> {
    with_archive_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_point_lights_runtime_tagged(
                cpu,
                w,
                h,
                env_h,
                tune,
                crate::room_gltf_punctual::RoomPunctualProfile::Standard,
                "archive.glb",
            )
        })
        .unwrap_or_default()
    })
}

pub fn archive_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<PointLight> {
    archive_embedded_point_lights_runtime_tagged(w, h, env_h, tune)
        .into_iter()
        .map(|t| t.light)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_tab_button_nodes_have_marker_bounds() {
        with_archive_glb_cpu(|opt| {
            let cpu = opt.expect("archive.glb should load in tests");
            for node in ARCHIVE_TAB_BUTTON_NODES {
                assert!(
                    cpu.marker_mesh_bounds_doc_for(node).is_some()
                        || cpu.markers.contains_key(node),
                    "missing marker data for {node}"
                );
            }
        });
    }

    #[test]
    fn archive_inspect_plaque_has_decal_host_bounds() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/3d/archive.glb");
        let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let cpu = load_archive_glb_from_bytes(&data).expect("decode archive.glb");
        let extents = archive_inspect_plaque_decal_extents_for(&cpu);
        assert!(
            extents[0] < 1.0,
            "inspect_plaque decal atlas should be portrait (u authored along local Y): {extents:?}"
        );
        assert!(
            cpu.marker_mesh_bounds_doc_for(INSPECT_PLAQUE).is_some()
                || crate::room_env_gltf::room_env_primitive_bounds_doc(
                    &cpu.environment_primitives,
                    INSPECT_PLAQUE,
                )
                .is_some(),
            "missing mesh bounds for inspect_plaque"
        );
    }
}
