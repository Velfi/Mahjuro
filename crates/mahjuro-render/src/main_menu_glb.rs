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

use chrono::Datelike;
use parking_lot::RwLock;

use glam::{Mat4, Vec3};

use crate::draw_cmd::CameraParams;
use crate::room_env_gltf::{self, RoomEnvWalkHooks, RoomMeshPolicy};
use crate::room_glb::{
    self, MarkerScreenRectParams, RoomEnvLightingTune, RoomGlbCpu, load_room_glb_from_bytes,
};
use crate::wgpu_renderer::{PointLight, SpotLight};
use crate::world_space::surface_anchor_from_world_xyz;

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
    crate::room_preload::join_main_menu_cpu_prefetch_blocking();
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
    drop(w);
    decode_main_menu_glb_into_cache();
}

/// Decode `main_menu.glb` into the process-wide CPU cache (main or prefetch thread).
/// Callers on the main thread should use [`with_main_menu_glb_cpu`] / `ensure` paths that
/// join an in-flight prefetch first — do not call this directly while prefetch is running.
pub fn decode_main_menu_glb_into_cache() {
    let mut w = MAIN_MENU_GLB_CPU.write();
    if matches!(
        &*w,
        MainMenuGlbCache::Ready(Some(cpu))
            if !room_glb::room_glb_cpu_needs_environment_mesh_reload(cpu)
                && !room_glb::room_glb_cpu_stale_environment_for_gpu_upload(cpu)
    ) {
        return;
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

/// Merged `rain_hit_*` soup handle for CPU rain raycasts (`Arc` clone — no triangle copy).
pub fn main_menu_rain_collision_mesh()
-> Option<std::sync::Arc<crate::room_env_gltf::RoomCollisionMesh>> {
    with_main_menu_glb_cpu(|opt| opt.and_then(|c| c.rain_collision_mesh().cloned()))
}

/// Room collision meshes for analytic punctual occlusion (roof, etc.).
pub fn main_menu_collision_meshes() -> Vec<crate::room_env_gltf::RoomCollisionMesh> {
    with_main_menu_glb_cpu(|opt| opt.map(|c| c.collision_meshes.clone()).unwrap_or_default())
}

/// Visible ground mesh node in [`main_menu.glb`](../../../assets/3d/main_menu.glb) (spawn fallback only).
pub const MAIN_MENU_RAIN_GROUND_NODE: &str = "ground";
const MAIN_MENU_FOG_GROUND_COLLIDER_NODE: &str = "rain_hit_ground";

/// Hub moon mesh in [`main_menu.glb`](../../../assets/3d/main_menu.glb) (`MoonObject` node).
/// Base-color albedo is phase-shaded in `room_glb.wgsl` from [`current_moon_phase`](crate::wgpu_renderer::current_moon_phase).
pub const MAIN_MENU_MOON_MESH_NODE: &str = "MoonObject";

/// Emissive star meshes (`star`, `star.001`, …) in [`main_menu.glb`](../../../assets/3d/main_menu.glb).
pub const MAIN_MENU_STAR_MESH_NODE_PREFIX: &str = "star";

#[inline]
pub fn is_main_menu_moon_env_node(name: &str) -> bool {
    name == MAIN_MENU_MOON_MESH_NODE
}

#[inline]
pub fn is_main_menu_star_env_node(name: &str) -> bool {
    name == MAIN_MENU_STAR_MESH_NODE_PREFIX || name.starts_with("star.")
}

/// Moon + star meshes that use the pride rainbow path in `room_glb.wgsl`
/// (moon: hard stripes; stars: smooth fade).
#[inline]
pub fn is_main_menu_rainbow_emissive_env_node(name: &str) -> bool {
    is_main_menu_moon_env_node(name) || is_main_menu_star_env_node(name)
}

/// June (local calendar) — default month for main-menu pride rainbow + moon quips.
#[inline]
pub fn main_menu_pride_month() -> bool {
    chrono::Local::now().month() == 6
}

/// Default pride-rainbow state at startup (on in June, off otherwise).
#[inline]
pub fn main_menu_pride_rainbow_default_enabled() -> bool {
    main_menu_pride_month()
}

/// Pride rainbow on main-menu moon / stars + starfield tint — driven by the moon
/// debug-menu toggle (`main_menu_pride_rainbow_debug` / overlay `pride_rainbow_debug`).
#[inline]
pub fn main_menu_pride_rainbow_active(enabled: bool) -> bool {
    enabled
}

/// World-space spawn column for CPU rain — union of every `rain_hit_*` shell (deck, rocks, roof, …).
pub fn main_menu_rain_hit_spawn_aabb(
    window_h: f32,
    env_scale: f32,
) -> Option<([f32; 3], [f32; 3])> {
    with_main_menu_glb_cpu(|opt| {
        let cpu = opt?;
        let center = cpu.environment_bounds_doc?.center();
        let bounds_doc =
            room_env_gltf::RoomEnvironmentBounds::from_collision_meshes(&cpu.rain_surface_meshes)
                .or_else(|| {
                room_env_gltf::room_env_primitive_bounds_doc(
                    &cpu.environment_primitives,
                    MAIN_MENU_RAIN_GROUND_NODE,
                )
            })?;
        Some(room_env_gltf::room_world_bounds_aabb_centered(
            bounds_doc, center, window_h, env_scale,
        ))
    })
}

/// World-space AABB for the visible main-menu room under `model`.
pub fn main_menu_environment_aabb_for_model(model: Mat4) -> Option<([f32; 3], [f32; 3])> {
    with_main_menu_glb_cpu(|opt| {
        let bounds = opt?.environment_bounds_doc?;
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for p in bounds.corners() {
            let w = model.transform_point3(p).to_array();
            for i in 0..3 {
                min[i] = min[i].min(w[i]);
                max[i] = max[i].max(w[i]);
            }
        }
        Some((min, max))
    })
}

/// World-space base height for main-menu low fog.
pub fn main_menu_height_fog_floor_z_for_model(model: Mat4) -> Option<f32> {
    with_main_menu_glb_cpu(|opt| {
        let cpu = opt?;
        let bounds = room_env_gltf::room_collision_mesh_bounds_doc(
            &cpu.rain_surface_meshes,
            MAIN_MENU_FOG_GROUND_COLLIDER_NODE,
        )
        .or_else(|| {
            room_env_gltf::room_env_primitive_bounds_doc(
                &cpu.environment_primitives,
                MAIN_MENU_RAIN_GROUND_NODE,
            )
        })?;
        let mut min_z = f32::INFINITY;
        for p in bounds.corners() {
            min_z = min_z.min(model.transform_point3(p).z);
        }
        min_z.is_finite().then_some(min_z)
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

/// True once `main_menu.glb` CPU decode finished (worker or main thread).
pub fn main_menu_cpu_decoded() -> bool {
    matches!(*MAIN_MENU_GLB_CPU.read(), MainMenuGlbCache::Ready(Some(_)))
}

/// True while environment mesh buffers are still on the CPU (ready for GPU upload).
pub fn main_menu_cpu_ready_for_gpu_upload() -> bool {
    with_main_menu_glb_cpu(|opt| {
        opt.is_some_and(|c| {
            !c.environment_primitives.is_empty() && !c.environment_primitives_released
        })
    })
}

pub fn release_main_menu_environment_cpu_sources_after_gpu_upload() {
    let mut g = MAIN_MENU_GLB_CPU.write();
    if let MainMenuGlbCache::Ready(Some(cpu)) = &mut *g {
        room_glb::release_room_environment_primitives_cpu(cpu);
    }
}

pub fn clear_main_menu_glb_cpu_cache() {
    *MAIN_MENU_GLB_CPU.write() = MainMenuGlbCache::Uninit;
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
    /// Moon (and other pick targets) need decoded mesh bounds for screen hit rects.
    /// `EnvironmentDraw` only merges bounds when `is_marker` is true.
    fn is_marker(&self, name: &str) -> bool {
        is_main_menu_moon_env_node(name)
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
            projection: crate::draw_cmd::CameraProjection::Perspective { fovy_deg: 52.0 },
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

/// Aspect-corrected moon disc radius from `moonlit_water.wgsl` (`moon_r`).
pub const MOONLIT_WATER_MOON_UV_RADIUS: f32 = 0.072;
/// Procedural moon center in `moonlit_water` UV space (0 = top, 1 = bottom).
pub const MOONLIT_WATER_MOON_CENTER_UV: [f32; 2] = [0.5, 0.28];

/// Screen-height fraction matching the procedural victory moon disc diameter.
#[inline]
pub fn moonlit_water_moon_diameter_screen_h() -> f32 {
    MOONLIT_WATER_MOON_UV_RADIUS * 2.0
}

/// World +Z offset so a recentered victory moon projects to [`MOONLIT_WATER_MOON_CENTER_UV`].
fn victory_moon_z_offset_for_uv_y(
    window_w: f32,
    window_h: f32,
    standoff: f32,
    fovy_deg: f32,
    uv_y: f32,
) -> f32 {
    let cam = crate::draw_cmd::CameraParams {
        eye: [0.0, -standoff, 0.0],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        projection: crate::draw_cmd::CameraProjection::Perspective { fovy_deg },
        clip_near: Some(0.01),
        clip_far: Some(window_h * crate::draw_cmd::SCENE_PERSPECTIVE_FAR_MUL),
    };
    let view_proj = cam.view_proj(window_w, window_h);

    let screen_y = |z: f32| {
        let clip = view_proj * glam::Vec4::new(0.0, 0.0, z, 1.0);
        let inv_w = 1.0 / clip.w.max(1e-6);
        let ny = clip.y * inv_w;
        (1.0 - (ny * 0.5 + 0.5)) * window_h
    };

    let target_sy = uv_y * window_h;
    let mut lo = -window_h;
    let mut hi = window_h;
    for _ in 0..32 {
        let mid = (lo + hi) * 0.5;
        if screen_y(mid) > target_sy {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

/// Extra scale applied to the victory run-summary moon after recentering (tune in Blender units).
pub const VICTORY_MOON_EXTRA_SCALE: f32 = 1.0;
/// Euler XYZ radians for the victory run-summary 3D moon.
pub const VICTORY_MOON_ROTATION_XYZ: [f32; 3] = [0.000000, 0.000000, -1.221730];

/// Camera + model delta for the victory screen's isolated `MoonObject` draw.
///
/// Recenters the hub moon at world origin so it sits in the middle of the view;
/// tune [`VICTORY_MOON_EXTRA_SCALE`] and [`VICTORY_MOON_ROTATION_XYZ`] as needed.
pub fn victory_summary_moon_setup(
    window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    rotation_xyz: [f32; 3],
) -> Option<(CameraParams, glam::Mat4)> {
    with_main_menu_glb_cpu(|opt| {
        let cpu = opt?;
        let moon = room_glb::room_node_mesh_center_world(
            cpu,
            window_h,
            env_height_scale,
            MAIN_MENU_MOON_MESH_NODE,
        )?;
        let moon_radius =
            room_node_mesh_radius_world(cpu, window_h, env_height_scale, MAIN_MENU_MOON_MESH_NODE)
                .unwrap_or(window_h * 0.015);

        let mut model_delta = glam::Mat4::from_translation(-moon);

        const FOVY_DEG: f32 = 45.0;
        let tan_half = (FOVY_DEG.to_radians() * 0.5).tan();
        let standoff = moon_radius * 3.5 + window_h * 0.02;
        let fit_scale =
            moonlit_water_moon_diameter_screen_h() * standoff * tan_half / moon_radius.max(1e-6);
        let scale = fit_scale * VICTORY_MOON_EXTRA_SCALE;
        model_delta = glam::Mat4::from_scale(glam::Vec3::splat(scale)) * model_delta;
        if rotation_xyz != [0.0, 0.0, 0.0] {
            model_delta = glam::Mat4::from_euler(
                glam::EulerRot::XYZ,
                rotation_xyz[0],
                rotation_xyz[1],
                rotation_xyz[2],
            ) * model_delta;
        }
        let z_offset = victory_moon_z_offset_for_uv_y(
            window_w,
            window_h,
            standoff,
            FOVY_DEG,
            MOONLIT_WATER_MOON_CENTER_UV[1],
        );
        model_delta =
            glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, z_offset)) * model_delta;

        Some((
            CameraParams {
                eye: [0.0, -standoff, 0.0],
                target: [0.0, 0.0, 0.0],
                up: [0.0, 0.0, 1.0],
                projection: crate::draw_cmd::CameraProjection::Perspective { fovy_deg: FOVY_DEG },
                clip_near: Some(0.01),
                clip_far: Some(window_h * crate::draw_cmd::SCENE_PERSPECTIVE_FAR_MUL),
            },
            model_delta,
        ))
    })
}

/// Close-up on the hub moon, facing it from the hub camera side (trailer-mode start pose).
pub fn main_menu_moon_trailer_start_camera(
    _window_w: f32,
    window_h: f32,
    env_height_scale: f32,
    end_cam: &CameraParams,
) -> Option<CameraParams> {
    with_main_menu_glb_cpu(|opt| {
        let cpu = opt?;
        let moon = room_glb::room_node_mesh_center_world(
            cpu,
            window_h,
            env_height_scale,
            MAIN_MENU_MOON_MESH_NODE,
        )?;
        let moon_radius =
            room_node_mesh_radius_world(cpu, window_h, env_height_scale, MAIN_MENU_MOON_MESH_NODE)
                .unwrap_or(window_h * 0.015);

        let end_eye = glam::Vec3::from_array(end_cam.eye);
        let to_moon = moon - end_eye;
        let dir = to_moon.normalize_or_zero();
        if dir.length_squared() < 1e-8 {
            return None;
        }

        let standoff = moon_radius * 1.4 + window_h * 0.006;
        let eye = moon - dir * standoff;

        Some(CameraParams {
            eye: eye.to_array(),
            target: moon.to_array(),
            up: end_cam.up,
            projection: crate::draw_cmd::CameraProjection::Perspective { fovy_deg: end_cam.fovy_deg() },
            clip_near: end_cam.clip_near,
            clip_far: end_cam.clip_far,
        })
    })
}

fn room_node_mesh_radius_world(
    cpu: &RoomGlbCpu,
    window_h: f32,
    height_scale: f32,
    node_name: &str,
) -> Option<f32> {
    let s = room_glb::room_env_world_scale(window_h, height_scale);
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
    if !any {
        return None;
    }
    let ext = (mx - mn) * s;
    Some(ext.max_element() * 0.5)
}

/// Screen hit rect for the emissive moon mesh (`MoonObject`), for hub click targets.
pub fn main_menu_moon_screen_hit_rect(w: f32, h: f32, env_h: f32) -> Option<[f32; 4]> {
    if !main_menu_room_draw_ready() {
        return None;
    }
    let cam = main_menu_camera_base(w, h, env_h);
    let scale = (w.min(h)) / 720.0;
    with_main_menu_glb_cpu(|opt| {
        let cpu = opt?;
        let params = MarkerScreenRectParams {
            win_w: w,
            win_h: h,
            cam: &cam,
            env_height_scale: env_h,
            cpu,
            node_name: MAIN_MENU_MOON_MESH_NODE,
            min_rw: (48.0 * scale).max(32.0),
            min_rh: (48.0 * scale).max(32.0),
        };
        room_glb::screen_rect_for_marker_mesh_bounds(&params).or_else(|| {
            let center =
                room_glb::room_node_mesh_center_world(cpu, h, env_h, MAIN_MENU_MOON_MESH_NODE)?;
            let (sx, sy) = cam.project_world_to_screen(w, h, center);
            let r = (56.0 * scale).max(40.0);
            Some([sx - r, sy - r, r * 2.0, r * 2.0])
        })
    })
}

/// Object3d anchor `[px, py, lift]` for the `light_doorway` punctual node in `main_menu.glb`.
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

pub fn main_menu_embedded_point_lights_runtime_tagged(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<crate::room_gltf_punctual::EmbeddedPointLightRuntime> {
    with_main_menu_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_point_lights_runtime_tagged(
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

pub fn main_menu_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<PointLight> {
    main_menu_embedded_point_lights_runtime_tagged(w, h, env_h, tune)
        .into_iter()
        .map(|t| t.light)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moon_object_has_screen_hit_bounds() {
        let bytes = include_bytes!("../../../assets/3d/main_menu.glb");
        let cpu = load_main_menu_glb_from_bytes(bytes).expect("decode main_menu.glb");
        assert!(
            cpu.marker_mesh_bounds_doc_for(MAIN_MENU_MOON_MESH_NODE)
                .is_some(),
            "MoonObject must merge mesh bounds for hub click targets"
        );
        let env_h = main_menu_env_height_scale(crate::room_glb::SHOP_ENV_HEIGHT_SCALE);
        let rect =
            main_menu_moon_screen_hit_rect(1280.0, 720.0, env_h).expect("projected moon hit rect");
        assert!(rect[2] > 0.0 && rect[3] > 0.0);
    }
}
