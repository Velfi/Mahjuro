//! [`staircase.glb`](../../../assets/3d/staircase.glb) — post-ordeal interstitial room.
//!
//! Shown after clearing an ordeal chamber, before the between-wing shop. Uses the same
//! GPU path as shop/hallway (`room_glb.wgsl`).

use std::sync::RwLock;

use glam::Vec3;

use crate::draw_cmd::CameraParams;
use crate::room_env_gltf::{RoomEnvWalkHooks, RoomMeshPolicy};
use crate::room_glb::{self, RoomEnvLightingTune, RoomGlbCpu, load_room_glb_from_bytes};
use crate::wgpu_renderer::{PointLight, SpotLight};

/// Multiplies `tile_seed` exposure in `room_glb.wgsl` for this room.
pub const STAIRCASE_ENV_LINEAR_EXPOSURE_MUL: f32 = 1.0;
pub const STAIRCASE_ENV_AMBIENT_SCALE_MIN: f32 = 0.0;

enum StaircaseGlbCache {
    Uninit,
    Ready(Option<Box<RoomGlbCpu>>),
}

static STAIRCASE_GLB_CPU: RwLock<StaircaseGlbCache> = RwLock::new(StaircaseGlbCache::Uninit);

fn ensure_staircase_glb_loaded() {
    let mut w = STAIRCASE_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    match &*w {
        StaircaseGlbCache::Uninit => {}
        StaircaseGlbCache::Ready(Some(cpu))
            if room_glb::room_glb_cpu_needs_environment_mesh_reload(cpu)
                || room_glb::room_glb_cpu_stale_environment_for_gpu_upload(cpu) =>
        {
            *w = StaircaseGlbCache::Uninit;
        }
        _ => return,
    }
    let ready = if let Some(file) = mahjuro_assets::asset_path::get("3d/staircase.glb") {
        match load_staircase_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::debug!(
                    "staircase.glb: {} marker(s), {} draw primitive(s)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                );
                Some(cpu)
            }
            Err(e) => panic!("staircase.glb failed to load: {e:#}"),
        }
    } else {
        panic!("staircase.glb not embedded; required when loading staircase room");
    };
    *w = StaircaseGlbCache::Ready(ready.map(Box::new));
}

pub fn staircase_glb_loaded() -> bool {
    with_staircase_glb_cpu(|o| o.is_some())
}

pub fn with_staircase_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_staircase_glb_loaded();
    let g = STAIRCASE_GLB_CPU.read().unwrap_or_else(|e| e.into_inner());
    match &*g {
        StaircaseGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        StaircaseGlbCache::Ready(None) => f(None),
        StaircaseGlbCache::Uninit => unreachable!(),
    }
}

pub fn release_staircase_environment_cpu_sources_after_gpu_upload() {
    let mut g = STAIRCASE_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if let StaircaseGlbCache::Ready(Some(cpu)) = &mut *g {
        room_glb::release_room_environment_primitives_cpu(cpu);
    }
}

#[derive(Copy, Clone)]
struct StaircaseRoomWalkHooks;

impl RoomEnvWalkHooks for StaircaseRoomWalkHooks {
    fn is_marker(&self, _name: &str) -> bool {
        false
    }

    fn mesh_policy(&self, _name: &str) -> RoomMeshPolicy {
        RoomMeshPolicy::EnvironmentDraw
    }

    fn log_asset_label(&self) -> &'static str {
        "staircase.glb"
    }
}

pub fn load_staircase_glb_from_bytes(data: &[u8]) -> anyhow::Result<RoomGlbCpu> {
    let mut cpu = load_room_glb_from_bytes(
        data,
        "gltf::import_slice(staircase.glb)",
        "staircase.glb has no scenes",
        &StaircaseRoomWalkHooks,
    )?;
    cpu.collision_meshes.clear();
    Ok(cpu)
}

fn staircase_embedded_camera_doc(cpu: &RoomGlbCpu) -> Option<room_glb::RoomGlbEmbeddedCamera> {
    let by = &cpu.embedded_cameras_by_name;
    by.get("default")
        .copied()
        .or(cpu.embedded_perspective_camera)
}

pub fn staircase_camera_from_glb_if_present(
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    with_staircase_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        staircase_embedded_camera_doc(cpu)
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

fn staircase_camera_resolve(
    w: f32,
    h: f32,
    env_h: f32,
    from_glb: Option<CameraParams>,
) -> CameraParams {
    with_staircase_glb_cpu(|opt| {
        let mut cam = from_glb.unwrap_or_else(|| CameraParams {
            eye: [0.0, -h * 1.1, h * 0.42],
            target: [0.0, h * 0.04, h * 0.22],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 52.0,
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
        }
        cam
    })
}

pub fn staircase_camera(w: f32, h: f32, env_h: f32) -> CameraParams {
    let from_glb = staircase_camera_from_glb_if_present(h, env_h);
    staircase_camera_resolve(w, h, env_h, from_glb)
}

pub fn staircase_glb_has_embedded_lights() -> bool {
    with_staircase_glb_cpu(|opt| {
        opt.is_some_and(crate::room_gltf_punctual::room_glb_has_embedded_lights)
    })
}

pub fn staircase_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<PointLight> {
    with_staircase_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_point_lights_runtime(
                cpu,
                w,
                h,
                env_h,
                tune,
                crate::room_gltf_punctual::RoomPunctualProfile::Standard,
                "staircase.glb",
            )
        })
        .unwrap_or_default()
    })
}

pub fn staircase_embedded_spot_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<SpotLight> {
    with_staircase_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_spot_lights_runtime(
                cpu,
                w,
                h,
                env_h,
                tune,
                "staircase.glb",
            )
        })
        .unwrap_or_default()
    })
}
