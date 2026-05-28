//! [`main_menu.glb`](../../../assets/3d/main_menu.glb) — main-menu waterfront room.
//!
//! Perspective camera node **`default`** selects framing when present; otherwise the first
//! embedded perspective camera or a bounds-fit fallback is used.
//!
//! Export **without Draco** (`KHR_draco_mesh_compression`). Use Blender glTF **Lighting Mode → Standard**
//! when using `KHR_lights_punctual`.
//!
//! Boolean `subtractor` meshes are culled at decode — see
//! [`crate::room_env_gltf::skip_room_env_authoring_mesh_node_name`].

use parking_lot::RwLock;

use glam::Vec3;

use crate::draw_cmd::CameraParams;
use crate::room_env_gltf::{self, RoomEnvWalkHooks, RoomMeshPolicy};
use crate::room_glb::{self, RoomEnvLightingTune, RoomGlbCpu, load_room_glb_from_bytes};
use crate::wgpu_renderer::{PointLight, SpotLight};
use crate::world_space::surface_anchor_from_world_xyz;

/// Linear HDR exposure multiplier when embedded punctual lights are active.
pub const MAIN_MENU_ENV_LINEAR_EXPOSURE_MUL: f32 = 1.0;

/// Hemispheric fill in `room_glb.wgsl` (`decal_atlas_uv.x`). Windowless interior — no sky ambient.
pub const MAIN_MENU_ENV_AMBIENT_SCALE_MIN: f32 = 0.0;

/// Doc→world height scale for [`main_menu.glb`](../../../assets/3d/main_menu.glb) vs [`crate::room_glb::SHOP_ENV_HEIGHT_SCALE`].
///
/// The hub waterfront is authored ~5× larger in glTF units than [`shop.glb`](../../../assets/3d/shop.glb)
/// (~43 vs ~8 units across the visible ground). Using the shop default makes the room enormous on
/// screen, so mip selection collapses to the smallest chain level and room textures look muddy.
pub const MAIN_MENU_ENV_HEIGHT_SCALE: f32 = 8.0 / 43.0;

/// Apply [`MAIN_MENU_ENV_HEIGHT_SCALE`] on top of the debug / global [`crate::room_glb::SHOP_ENV_HEIGHT_SCALE`].
#[inline]
pub fn main_menu_env_height_scale(debug_room_gltf_height_scale: f32) -> f32 {
    debug_room_gltf_height_scale
        * (MAIN_MENU_ENV_HEIGHT_SCALE / crate::room_glb::SHOP_ENV_HEIGHT_SCALE)
}

enum MainMenuGlbCache {
    Uninit,
    Ready(Option<Box<RoomGlbCpu>>),
}

static MAIN_MENU_GLB_CPU: RwLock<MainMenuGlbCache> = RwLock::new(MainMenuGlbCache::Uninit);

fn ensure_main_menu_glb_loaded() {
    let mut w = MAIN_MENU_GLB_CPU.write();
    match &*w {
        MainMenuGlbCache::Uninit => {}
        MainMenuGlbCache::Ready(Some(cpu))
            if room_glb::room_glb_cpu_needs_environment_mesh_reload(cpu) =>
        {
            *w = MainMenuGlbCache::Uninit;
        }
        _ => return,
    }
    let ready = if let Some(file) = mahjuro_assets::asset_path::get("3d/main_menu.glb") {
        match load_main_menu_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                if cpu.rain_surface_meshes.is_empty() {
                    log::warn!(
                        "main_menu.glb: no rain_hit_* collision meshes — CPU rain splashes need invisible shells named rain_hit_* (export to assets/3d/main_menu.glb)"
                    );
                } else {
                    let rain_tris = cpu
                        .rain_surface_merged
                        .as_ref()
                        .map(|m| m.triangles.len())
                        .unwrap_or(0);
                    log::info!(
                        "main_menu.glb: {} rain surface mesh(es) → {} merged triangle(s): {:?}",
                        cpu.rain_surface_meshes.len(),
                        rain_tris,
                        cpu.rain_surface_meshes
                            .iter()
                            .map(|m| m.node_name.as_str())
                            .collect::<Vec<_>>(),
                    );
                }
                log::debug!(
                    "main_menu.glb: {} marker(s), {} draw primitive(s), {} punctual occluder mesh(es), named cameras: {:?}",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                    cpu.collision_meshes.len(),
                    cpu.embedded_cameras_by_name.keys().collect::<Vec<_>>(),
                );
                crate::room_gltf_punctual::log_main_menu_punctual_light_nodes(&cpu);
                Some(cpu)
            }
            Err(e) => panic!("main_menu.glb failed to load: {e:#}"),
        }
    } else {
        panic!("main_menu.glb not embedded; required at renderer init");
    };
    *w = MainMenuGlbCache::Ready(ready.map(Box::new));
}

/// Env model matrix for rain collision (same centering/scale as the drawn room).
pub fn main_menu_rain_env_model_matrix(window_h: f32, env_scale: f32) -> Option<glam::Mat4> {
    with_main_menu_glb_cpu(|opt| {
        opt.map(|cpu| room_glb::room_env_model_matrix_from_cpu(window_h, env_scale, cpu))
    })
}

/// Decoded `rain_hit_*` triangle soups (empty until meshes are authored in Blender).
pub fn main_menu_rain_surface_meshes() -> Vec<crate::room_env_gltf::RoomCollisionMesh> {
    with_main_menu_glb_cpu(|opt| {
        opt.map(|c| c.rain_surface_meshes.clone())
            .unwrap_or_default()
    })
}

/// Merged `rain_hit_*` soup clone for CPU rain raycasts.
pub fn main_menu_rain_collision_mesh() -> Option<crate::room_env_gltf::RoomCollisionMesh> {
    with_main_menu_glb_cpu(|opt| opt.and_then(|c| c.rain_collision_mesh().cloned()))
}

/// Room collision meshes for analytic punctual occlusion (roof, etc.).
pub fn main_menu_collision_meshes() -> Vec<crate::room_env_gltf::RoomCollisionMesh> {
    with_main_menu_glb_cpu(|opt| {
        opt.map(|c| c.collision_meshes.clone()).unwrap_or_default()
    })
}

/// Visible ground mesh node in [`main_menu.glb`](../../../assets/3d/main_menu.glb) (spawn fallback only).
pub const MAIN_MENU_RAIN_GROUND_NODE: &str = "ground";

/// World-space spawn column for CPU rain — union of every `rain_hit_*` shell (deck, rocks, roof, …).
pub fn main_menu_rain_hit_spawn_aabb(
    window_h: f32,
    env_scale: f32,
) -> Option<([f32; 3], [f32; 3])> {
    with_main_menu_glb_cpu(|opt| {
        let cpu = opt?;
        let center = cpu.environment_bounds_doc?.center();
        let bounds_doc = room_env_gltf::RoomEnvironmentBounds::from_collision_meshes(
            &cpu.rain_surface_meshes,
        )
        .or_else(|| {
            room_env_gltf::room_env_primitive_bounds_doc(
                &cpu.environment_primitives,
                MAIN_MENU_RAIN_GROUND_NODE,
            )
        })?;
        Some(room_env_gltf::room_world_bounds_aabb_centered(
            bounds_doc,
            center,
            window_h,
            env_scale,
        ))
    })
}

/// `true` when `main_menu.glb` loaded and the hub can draw the 3D room.
///
/// After init, CPU mesh buffers may be released while GPU draws remain; bounds
/// stay populated so do not gate on [`RoomGlbCpu::environment_primitives`].
pub fn main_menu_room_draw_ready() -> bool {
    with_main_menu_glb_cpu(|opt| {
        opt.is_some_and(|c| {
            c.environment_bounds_doc.is_some() || !c.environment_primitives.is_empty()
        })
    })
}

pub fn with_main_menu_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_main_menu_glb_loaded();
    let g = MAIN_MENU_GLB_CPU.read();
    match &*g {
        MainMenuGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        MainMenuGlbCache::Ready(None) => f(None),
        MainMenuGlbCache::Uninit => {
            log::warn!("main_menu.glb cache still Uninit after ensure — treating as absent");
            f(None)
        }
    }
}

pub fn release_main_menu_environment_cpu_sources_after_gpu_upload() {
    let mut g = MAIN_MENU_GLB_CPU.write();
    if let MainMenuGlbCache::Ready(Some(cpu)) = &mut *g {
        room_glb::release_room_environment_primitives_cpu(cpu);
    }
}

/// Meshes that block embedded doorway / porch punctuals in `room_glb.wgsl` (analytic AABB rays).
#[inline]
fn is_main_menu_punctual_occluder_mesh(name: &str) -> bool {
    matches!(
        name,
        "rooflet" | "pCube22_M_roof_0" | "pCube22_aiStandardSurface8_0"
    )
}

#[derive(Copy, Clone)]
struct MainMenuRoomWalkHooks;

impl RoomEnvWalkHooks for MainMenuRoomWalkHooks {
    fn is_marker(&self, _name: &str) -> bool {
        false
    }

    fn mesh_policy(&self, name: &str) -> RoomMeshPolicy {
        if name.starts_with("rain_hit") {
            RoomMeshPolicy::RainSurfaceCollision
        } else if is_main_menu_punctual_occluder_mesh(name) {
            RoomMeshPolicy::EnvironmentDrawWithCollision
        } else {
            RoomMeshPolicy::EnvironmentDraw
        }
    }

    fn log_asset_label(&self) -> &'static str {
        "main_menu.glb"
    }
}

pub fn load_main_menu_glb_from_bytes(data: &[u8]) -> anyhow::Result<RoomGlbCpu> {
    load_room_glb_from_bytes(
        data,
        "gltf::import_slice(main_menu.glb)",
        "main_menu.glb has no scenes",
        &MainMenuRoomWalkHooks,
    )
}

#[inline]
fn main_menu_embedded_camera_doc(cpu: &RoomGlbCpu) -> Option<room_glb::RoomGlbEmbeddedCamera> {
    cpu.embedded_cameras_by_name
        .get("default")
        .copied()
        .or(cpu.embedded_perspective_camera)
}

pub fn main_menu_camera_from_glb_if_present(
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    with_main_menu_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        main_menu_embedded_camera_doc(cpu)
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

fn main_menu_camera_resolve(
    w: f32,
    h: f32,
    env_h: f32,
    from_glb: Option<CameraParams>,
) -> CameraParams {
    with_main_menu_glb_cpu(|opt| {
        let mut cam = from_glb.unwrap_or_else(|| CameraParams {
            eye: [0.0, -h * 1.2, h * 0.45],
            target: [0.0, h * 0.04, h * 0.14],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 52.0,
            clip_near: None,
            clip_far: None,
        });
        // Auto-fit widens vertical FOV past the authored glTF value; keep embedded
        // eye / target / up / fovy intact (same policy as shop).
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

pub fn main_menu_camera_base(w: f32, h: f32, env_h: f32) -> CameraParams {
    let from_glb = main_menu_camera_from_glb_if_present(h, env_h);
    main_menu_camera_resolve(w, h, env_h, from_glb)
}

pub fn main_menu_glb_has_embedded_lights() -> bool {
    with_main_menu_glb_cpu(|opt| {
        opt.is_some_and(crate::room_gltf_punctual::room_glb_has_embedded_lights)
    })
}

/// Object3d anchor `[px, py, lift]` for the `light_doorway` punctual node in `main_menu.glb`.
#[allow(dead_code)]
pub fn main_menu_light_door_object3d_anchor(w: f32, h: f32, env_h: f32) -> Option<[f32; 3]> {
    with_main_menu_glb_cpu(|opt| {
        let cpu = opt?;
        let light = cpu
            .embedded_point_lights
            .iter()
            .find(|l| l.node_name == "light_doorway")?;
        let s = room_glb::room_env_world_scale(h, env_h);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        let world = (light.pos_doc - center_doc) * s;
        Some(surface_anchor_from_world_xyz(w, h, world))
    })
}

pub fn main_menu_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<PointLight> {
    with_main_menu_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_point_lights_runtime(
                cpu,
                w,
                h,
                env_h,
                tune,
                crate::room_gltf_punctual::RoomPunctualProfile::MainMenu,
                "main_menu.glb",
            )
        })
        .unwrap_or_default()
    })
}

pub fn main_menu_embedded_spot_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<SpotLight> {
    with_main_menu_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_spot_lights_runtime(
                cpu,
                w,
                h,
                env_h,
                tune,
                "main_menu.glb",
            )
        })
        .unwrap_or_default()
    })
}
