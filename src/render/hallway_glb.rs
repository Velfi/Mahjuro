//! [`hallway.glb`](../../../assets/hallway.glb) — pick-blind hallway room.
//!
//! Marker object names (Blender → glTF):
//! - `btn_play_round` — commit to the upcoming blind (same as legacy Play altar).
//! - `btn_skip_round` — skip for the tribute tag when allowed (non-boss).
//!
//! Decodes through [`crate::render::room_env_gltf`]; decoded layout matches [`crate::render::shop_glb::RoomGlbCpu`]
//! for the shared GPU path (`shop_glb.wgsl` / embedded lights).

use std::sync::RwLock;

use glam::Vec3;

use crate::render::draw_cmd::CameraParams;
use crate::render::room_env_gltf::{RoomEnvWalkHooks, RoomMeshPolicy, glb_punctual_range_world_upload};
use crate::render::shop_glb::{
    self, RoomGlbCpu, ShopEnvLightingTune, load_room_glb_from_bytes,
};
use crate::render::world_space::surface_anchor_from_world_xyz;
use crate::render::wgpu_renderer::{MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS, PointLight, SpotLight};

/// glTF node names for pick-blind actions (must match Blender objects).
pub const BTN_PLAY_ROUND: &str = "btn_play_round";
pub const BTN_SKIP_ROUND: &str = "btn_skip_round";

/// Applied in [`crate::render::wgpu_renderer::WgpuRenderer`] when writing hallway env uniforms:
/// multiplies `tile_seed` on top of the shared shop/storeroom exposure path.
pub const HALLWAY_ENV_LINEAR_EXPOSURE_MUL: f32 = 2.35;

/// Minimum `decal_atlas_uv.x` (hemispheric fill in `shop_glb.wgsl`) for this room; `max` with debug tune.
pub const HALLWAY_ENV_AMBIENT_SCALE_MIN: f32 = 0.085;

enum HallwayGlbCache {
    Uninit,
    Ready(Option<RoomGlbCpu>),
}

static HALLWAY_GLB_CPU: RwLock<HallwayGlbCache> = RwLock::new(HallwayGlbCache::Uninit);

fn ensure_hallway_glb_loaded() {
    let mut w = HALLWAY_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if !matches!(*w, HallwayGlbCache::Uninit) {
        return;
    }
    let ready = if let Some(file) = crate::asset_path::get("hallway.glb") {
        match load_hallway_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::debug!(
                    "hallway.glb: {} marker(s), {} draw primitive(s)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                );
                Some(cpu)
            }
            Err(e) => {
                log::error!("hallway.glb failed to load: {e:#}");
                None
            }
        }
    } else {
        log::warn!("hallway.glb not embedded");
        None
    };
    *w = HallwayGlbCache::Ready(ready);
}

/// Read-only access to decoded hallway data.
pub fn with_hallway_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_hallway_glb_loaded();
    let g = HALLWAY_GLB_CPU.read().unwrap_or_else(|e| e.into_inner());
    match &*g {
        HallwayGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        HallwayGlbCache::Ready(None) => f(None),
        HallwayGlbCache::Uninit => unreachable!(),
    }
}

/// Drops CPU mesh/texture RAM after GPU upload (same contract as shop).
pub fn release_hallway_environment_cpu_sources_after_gpu_upload() {
    let mut g = HALLWAY_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if let HallwayGlbCache::Ready(Some(cpu)) = &mut *g {
        shop_glb::release_room_environment_primitives_cpu(cpu);
    }
}

#[inline]
fn is_hallway_marker_name(name: &str) -> bool {
    matches!(name, BTN_PLAY_ROUND | BTN_SKIP_ROUND)
}

#[derive(Copy, Clone)]
struct HallwayRoomWalkHooks;

impl RoomEnvWalkHooks for HallwayRoomWalkHooks {
    fn is_marker(&self, name: &str) -> bool {
        is_hallway_marker_name(name)
    }

    fn mesh_policy(&self, _name: &str) -> RoomMeshPolicy {
        RoomMeshPolicy::EnvironmentDraw
    }

    fn log_asset_label(&self) -> &'static str {
        "hallway.glb"
    }
}

pub fn load_hallway_glb_from_bytes(data: &[u8]) -> anyhow::Result<RoomGlbCpu> {
    let mut cpu = load_room_glb_from_bytes(
        data,
        "gltf::import_slice(hallway.glb)",
        "hallway.glb has no scenes",
        &HallwayRoomWalkHooks,
    )?;
    cpu.collision_meshes.clear();
    Ok(cpu)
}

/// World-space marker position (same centering + scale as uploaded hallway mesh).
pub fn hallway_marker_world(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> Option<Vec3> {
    let t = shop_glb::marker_translation(cpu, name)?;
    let s = shop_glb::shop_env_world_scale(window_h, env_height_scale);
    Some(t * s)
}

pub fn hallway_camera_from_glb_if_present(
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    with_hallway_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        cpu.embedded_perspective_camera
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

/// Camera for pick-blind: embedded perspective when present, else fit bounds (legacy shrine framing).
pub fn hallway_camera_base(w: f32, h: f32, env_h: f32) -> CameraParams {
    let from_glb = hallway_camera_from_glb_if_present(h, env_h);
    let cam = from_glb.unwrap_or_else(|| CameraParams {
        eye: [0.0, -h * 1.25, h * 0.50],
        target: [0.0, h * 0.05, h * 0.18],
        up: [0.0, 0.0, 1.0],
        fovy_deg: 55.0,
    });
    if from_glb.is_some() {
        return cam;
    }
    with_hallway_glb_cpu(|opt| {
        if let Some(cpu) = opt {
            let corners = shop_glb::shop_world_bounds_corners_centered(h, env_h, cpu);
            shop_glb::shop_camera_fit_fovy_for_corners(w, h, cam, &corners, 0.94)
        } else {
            cam
        }
    })
}

pub fn hallway_glb_has_embedded_lights() -> bool {
    with_hallway_glb_cpu(|opt| {
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

/// glTF punctual points merged into [`crate::render::draw_cmd::SceneLighting::punctual`] (hallway room).
pub fn hallway_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &ShopEnvLightingTune,
) -> Vec<PointLight> {
    with_hallway_glb_cpu(|opt| {
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
                "hallway.glb: {} point lights exceed budget ({}) — truncating",
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

/// glTF spot lights for [`UiFrame::spot_lights`].
pub fn hallway_embedded_spot_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &ShopEnvLightingTune,
) -> Vec<SpotLight> {
    if !hallway_glb_has_embedded_lights() {
        return Vec::new();
    }
    with_hallway_glb_cpu(|opt| {
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
                "hallway.glb: {} spot lights exceed {} — truncating",
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

#[cfg(test)]
mod tests {
    use super::load_hallway_glb_from_bytes;

    /// Pick-blind room (`hallway.glb`) — documents how many environment primitives carry glTF
    /// emissive; re-run after authoring so the count reflects `emissiveTexture` / factor.
    #[test]
    fn pick_blind_room_emissive_material_summary() {
        let data = match crate::asset_path::get("hallway.glb") {
            Some(f) => f.data,
            None => {
                eprintln!(
                    "skip pick_blind_room_emissive_material_summary: no hallway.glb (bake packs or set MAHJURO_ASSETS)"
                );
                return;
            }
        };
        let cpu = load_hallway_glb_from_bytes(&data).expect("hallway.glb decode");
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
            "hallway.glb (pick-blind): {} env primitive(s), {} with emissive texture, {} with non-zero emissive factor",
            cpu.environment_primitives.len(),
            with_tex,
            with_factor
        );
    }
}
