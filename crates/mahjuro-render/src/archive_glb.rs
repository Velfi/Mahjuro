//! [`archive.glb`](../../../assets/3d/archive.glb) — Archive (`CollectionScene`) room.
//!
//! ## Node names (Blender object names → glTF nodes)
//!
//! - `sign_description_left` / `sign_description_right` — description boards; runtime draws one
//!   mesh (see `archive_env_skip_description_prim`) based on cursor so the active board is away
//!   from the pointer; catalog copy is CPU-rasterized into a shared decal atlas and composited in
//!   `room_glb.wgsl` on those meshes (vertex `COLOR_0.a = 2` tag from `decode_env_primitive`).
//! - `archive_spawn_item.001` … `archive_spawn_item.021` — 21 item anchors (3×7 window into the tab catalogue).
//! - `btn_relics_tab`, `btn_zodiacs_tab`, `btn_bosses_tab`, `btn_talismans_tab`, `btn_chronicle_tab` —
//!   section tabs (meshes draw; hit rects project mesh AABBs like shop `exit_btn`).
//! - `btn_main_menu`, `btn_switch_save` — title-bar chrome.
//! - `btn_page_left`, `btn_page_right` — cabinet page step (hidden at first/last page).
//! - `section_buttons_left_bound` / `section_buttons_right_bound` — legacy bounds (collision only when present).
//! - `archive_spawn_focused_item` — large featured / inspect anchor.
//!
//! **Description copy:** name/body text is **CPU-rasterized** in [`CollectionScene`](../../scenes/collection.rs)
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

use std::sync::RwLock;

use glam::{Mat4, Vec3};

use crate::draw_cmd::CameraParams;
use crate::room_env_gltf::{RoomEnvWalkHooks, RoomMeshPolicy};
use crate::room_glb::{self, RoomEnvLightingTune, RoomGlbCpu, load_room_glb_from_bytes};
use crate::wgpu_renderer::{PointLight, SpotLight};

pub const SIGN_DESCRIPTION_LEFT: &str = "sign_description_left";
pub const SIGN_DESCRIPTION_RIGHT: &str = "sign_description_right";
pub const SECTION_BUTTONS_LEFT_BOUND: &str = "section_buttons_left_bound";
pub const SECTION_BUTTONS_RIGHT_BOUND: &str = "section_buttons_right_bound";
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

/// Tab button glTF nodes in [`crate::scenes::collection::TABS`] order (Relics → Talismans → Yaku → Bosses → Chronicle).
pub const ARCHIVE_TAB_BUTTON_NODES: [&str; 5] = [
    BTN_RELICS_TAB,
    BTN_TALISMANS_TAB,
    BTN_ZODIACS_TAB,
    BTN_BOSSES_TAB,
    BTN_CHRONICLE_TAB,
];

/// Whether this archive room primitive casts into the directional shadow map (live or bake).
///
/// Archive never casts room GLB into the directional map — contact is punctual-only.
#[inline]
pub fn archive_prim_casts_room_shadow(_node_name: Option<&str>) -> bool {
    false
}

/// Fallback host extents for [`crate::decal::decal_dimensions`] when the archive `.glb`
/// is missing. Live archive decal sizing reads the actual sign-face aspect via
/// [`archive_sign_description_decal_extents`].
pub const ARCHIVE_DESCRIPTION_DECAL_HOST_EXTENTS: [f32; 3] = [1.0, 1.0, 1.0];

/// Read the `sign_description_*` mesh's face aspect from a loaded archive `.glb` and pack it
/// into the `[long, short, thin]` host extents that [`crate::decal::decal_dimensions`]
/// expects. Returns [`ARCHIVE_DESCRIPTION_DECAL_HOST_EXTENTS`] when the asset is unavailable so
/// the rasterized decal matches the rectangular sign and does not stretch / squish glyphs.
///
/// Takes `cpu` by reference rather than re-entering [`with_archive_glb_cpu`] because callers
/// (`build.rs`, `sync_archive_description_decal_texture`) are typically already inside that
/// closure — recursive read-locks on `std::sync::RwLock` are UB on macOS pthreads and deadlock
/// in practice.
pub fn archive_sign_description_decal_extents_for(cpu: &RoomGlbCpu) -> [f32; 3] {
    let Some(bounds) = cpu
        .marker_mesh_bounds_doc_for(SIGN_DESCRIPTION_LEFT)
        .or_else(|| cpu.marker_mesh_bounds_doc_for(SIGN_DESCRIPTION_RIGHT))
    else {
        return ARCHIVE_DESCRIPTION_DECAL_HOST_EXTENTS;
    };
    let diag = bounds.max - bounds.min;
    let mut e = [diag.x.abs(), diag.y.abs(), diag.z.abs()];
    // Sort ascending so e[2] = longest face edge, e[1] = shorter face edge, e[0] = thickness.
    e.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let long = e[2].max(1e-6);
    let short = e[1].max(1e-6);
    // `decal_dimensions(Fit)` does `extents[0] / extents[1].max(1.0)` — normalize the short
    // edge to exactly 1.0 so doc-space units (which can be < 1) still yield the true aspect.
    [long / short, 1.0, 1.0]
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

/// Linear HDR exposure multiplier for `archive.glb` (applied when embedded punctual lights are active).
pub const ARCHIVE_ENV_LINEAR_EXPOSURE_MUL: f32 = 1.0;

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
    Ready(Option<Box<RoomGlbCpu>>),
}

static ARCHIVE_GLB_CPU: RwLock<ArchiveGlbCache> = RwLock::new(ArchiveGlbCache::Uninit);

fn ensure_archive_glb_loaded() {
    let mut w = ARCHIVE_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
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
        room_glb::release_room_environment_primitives_cpu(cpu);
    }
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
                | SECTION_BUTTONS_LEFT_BOUND
                | SECTION_BUTTONS_RIGHT_BOUND
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
        if matches!(name, SIGN_DESCRIPTION_LEFT | SIGN_DESCRIPTION_RIGHT)
            || is_archive_button_node(name)
        {
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
    // Section bounds (when present) keep collision only; tab/chrome buttons draw like shop controls.
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
    let t = room_glb::marker_translation(cpu, name)?;
    let s = room_glb::room_env_world_scale(window_h, env_height_scale);
    Some(t * s)
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
            fovy_deg: 50.0,
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

pub fn archive_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<PointLight> {
    with_archive_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_point_lights_runtime(
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

pub fn archive_embedded_spot_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<SpotLight> {
    with_archive_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_spot_lights_runtime(
                cpu,
                w,
                h,
                env_h,
                tune,
                "archive.glb",
            )
        })
        .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether this archive mesh skips directional shadows (matches decode in [`room_env_gltf`]:
    /// every shell primitive except description boards).
    fn archive_env_skips_directional_room_shadow(node_name: &str) -> bool {
        !matches!(node_name, SIGN_DESCRIPTION_LEFT | SIGN_DESCRIPTION_RIGHT)
    }

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
    fn archive_shadow_caster_receiver_split() {
        assert!(!archive_prim_casts_room_shadow(Some("main_fixture")));
        assert!(!archive_prim_casts_room_shadow(Some(SIGN_DESCRIPTION_LEFT)));
        assert!(!archive_prim_casts_room_shadow(Some("Cubby")));
        assert!(archive_env_skips_directional_room_shadow(
            "text_scene_title"
        ));
        assert!(archive_env_skips_directional_room_shadow(BTN_PAGE_RIGHT));
        assert!(archive_env_skips_directional_room_shadow("main_fixture"));
        assert!(!archive_env_skips_directional_room_shadow(
            SIGN_DESCRIPTION_LEFT
        ));
    }
}
