//! [`shadow_test_room.glb`](../../../assets/3d/shadow_test_room.glb) - debug-only room
//! used by the Shadow & AO lab. It uses the shared room GLB GPU path but is not
//! part of the normal startup prefetch or offline bake room set.

use parking_lot::RwLock;

use glam::Vec3;

use crate::draw_cmd::CameraParams;
use crate::room_env_gltf::{RoomEnvWalkHooks, RoomMeshPolicy};
use crate::room_glb::{self, RoomEnvLightingTune, RoomGlbCpu, load_room_glb_from_bytes};
use crate::wgpu_renderer::PointLight;

enum ShadowTestRoomGlbCache {
    Uninit,
    Ready(Option<Box<RoomGlbCpu>>),
}

static SHADOW_TEST_ROOM_GLB_CPU: RwLock<ShadowTestRoomGlbCache> =
    RwLock::new(ShadowTestRoomGlbCache::Uninit);

fn ensure_shadow_test_room_glb_loaded() {
    let mut w = SHADOW_TEST_ROOM_GLB_CPU.write();
    match &*w {
        ShadowTestRoomGlbCache::Uninit => {}
        ShadowTestRoomGlbCache::Ready(Some(cpu))
            if room_glb::room_glb_cpu_needs_environment_mesh_reload(cpu) =>
        {
            *w = ShadowTestRoomGlbCache::Uninit;
        }
        _ => return,
    }
    let ready = if let Some(file) = mahjuro_assets::asset_path::get("3d/shadow_test_room.glb") {
        match load_shadow_test_room_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::debug!(
                    "shadow_test_room.glb: {} marker(s), {} draw primitive(s)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                );
                Some(cpu)
            }
            Err(e) => panic!("shadow_test_room.glb failed to load: {e:#}"),
        }
    } else {
        panic!("shadow_test_room.glb not embedded; required by the shadow lab test room");
    };
    *w = ShadowTestRoomGlbCache::Ready(ready.map(Box::new));
}

pub fn shadow_test_room_glb_loaded() -> bool {
    with_shadow_test_room_glb_cpu(|o| o.is_some())
}

pub fn decode_shadow_test_room_glb_into_cache() {
    ensure_shadow_test_room_glb_loaded();
}

pub fn with_shadow_test_room_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_shadow_test_room_glb_loaded();
    let g = SHADOW_TEST_ROOM_GLB_CPU.read();
    match &*g {
        ShadowTestRoomGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        ShadowTestRoomGlbCache::Ready(None) => f(None),
        ShadowTestRoomGlbCache::Uninit => unreachable!(),
    }
}

pub fn release_shadow_test_room_environment_cpu_sources_after_gpu_upload() {
    let mut g = SHADOW_TEST_ROOM_GLB_CPU.write();
    if let ShadowTestRoomGlbCache::Ready(Some(cpu)) = &mut *g {
        room_glb::release_room_environment_primitives_cpu(cpu);
    }
}

pub fn shadow_test_room_cpu_ready_for_gpu_upload() -> bool {
    let g = SHADOW_TEST_ROOM_GLB_CPU.read();
    match &*g {
        ShadowTestRoomGlbCache::Ready(Some(cpu)) => {
            !cpu.environment_primitives.is_empty() && !cpu.environment_primitives_released
        }
        _ => false,
    }
}

pub fn clear_shadow_test_room_glb_cpu_cache() {
    *SHADOW_TEST_ROOM_GLB_CPU.write() = ShadowTestRoomGlbCache::Uninit;
}

#[derive(Copy, Clone)]
struct ShadowTestRoomWalkHooks;

impl RoomEnvWalkHooks for ShadowTestRoomWalkHooks {
    fn is_marker(&self, _name: &str) -> bool {
        false
    }

    fn mesh_policy(&self, _name: &str) -> RoomMeshPolicy {
        RoomMeshPolicy::EnvironmentDraw
    }

    fn log_asset_label(&self) -> &'static str {
        "shadow_test_room.glb"
    }
}

pub fn load_shadow_test_room_glb_from_bytes(data: &[u8]) -> anyhow::Result<RoomGlbCpu> {
    let mut cpu = load_room_glb_from_bytes(
        data,
        "gltf::import_slice(shadow_test_room.glb)",
        "shadow_test_room.glb has no scenes",
        &ShadowTestRoomWalkHooks,
    )?;
    cpu.collision_meshes.clear();
    Ok(cpu)
}

fn shadow_test_room_embedded_camera_doc(
    cpu: &RoomGlbCpu,
) -> Option<room_glb::RoomGlbEmbeddedCamera> {
    let by = &cpu.embedded_cameras_by_name;
    by.get("default")
        .copied()
        .or(cpu.embedded_perspective_camera)
}

pub fn shadow_test_room_camera_from_glb_if_present(
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    with_shadow_test_room_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        shadow_test_room_embedded_camera_doc(cpu)
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

pub fn shadow_test_room_camera(w: f32, h: f32, env_h: f32) -> CameraParams {
    let from_glb = shadow_test_room_camera_from_glb_if_present(h, env_h);
    with_shadow_test_room_glb_cpu(|opt| {
        let mut cam = from_glb.unwrap_or_else(|| CameraParams {
            eye: [0.0, -h * 1.1, h * 0.5],
            target: [0.0, h * 0.05, h * 0.2],
            up: [0.0, 0.0, 1.0],
            projection: crate::draw_cmd::CameraProjection::Perspective { fovy_deg: 52.0 },
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

pub fn shadow_test_room_glb_has_embedded_lights() -> bool {
    with_shadow_test_room_glb_cpu(|opt| {
        opt.is_some_and(crate::room_gltf_punctual::room_glb_has_embedded_lights)
    })
}

pub fn shadow_test_room_embedded_point_lights_runtime_tagged(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<crate::room_gltf_punctual::EmbeddedPointLightRuntime> {
    with_shadow_test_room_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_point_lights_runtime_tagged(
                cpu,
                w,
                h,
                env_h,
                tune,
                crate::room_gltf_punctual::RoomPunctualProfile::Standard,
                "shadow_test_room.glb",
            )
        })
        .unwrap_or_default()
    })
}

pub fn shadow_test_room_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<PointLight> {
    shadow_test_room_embedded_point_lights_runtime_tagged(w, h, env_h, tune)
        .into_iter()
        .map(|t| t.light)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_shadow_test_room_is_shadow_ao_comparison_fixture() {
        let bytes = include_bytes!("../../../assets/3d/shadow_test_room.glb");
        let cpu = load_shadow_test_room_glb_from_bytes(bytes).unwrap();
        assert_eq!(cpu.environment_primitives.len(), 16);
        assert!(cpu.embedded_cameras_by_name.contains_key("default"));
        assert_eq!(cpu.embedded_point_lights.len(), 1);
        assert_eq!(
            cpu.embedded_point_lights[0].node_name,
            "light_shadow_ao_comparison"
        );
        let node_names: std::collections::BTreeSet<&str> = cpu
            .environment_primitives
            .iter()
            .filter_map(|p| p.gltf_node_name.as_deref())
            .collect();
        for required in [
            "thick_light_blocking_roof",
            "back_wall_shadow_receiver_panel",
            "shadowed_receiver_under_roof",
            "lit_receiver_open_apron",
            "floor_shadow_fin",
            "wall_shadow_bar",
            "grounded_contact_block",
            "raised_gap_block",
            "corner_contact_step_low",
            "corner_contact_step_high",
        ] {
            assert!(node_names.contains(required), "missing node {required}");
        }
    }
}
