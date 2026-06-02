//! Xbox-style projected depth maps: one orthographic view per punctual shadow
//! caster, aimed at the room center. Punctual-only; spot shadows are unsupported.
//!
//! World space is **Z-up** (table in XY, +Z up). Light positions must match
//! the GPU punctual buffer — see [`punctual_light_world`].

use glam::{Mat4, Vec3};

use crate::draw_cmd::{CameraParams, ScenePunctualLight, UiFrame};
use crate::punctual_shadow_policy::punctual_light_casts_shadow;
use crate::room_env_gltf::{RoomEnvironmentBounds, room_world_bounds_corners_centered};
use crate::room_glb::room_env_world_scale;
use crate::wgpu_renderer::{ActiveRoomEnv, MAX_POINT_LIGHTS};
use crate::world_space::{pixel_to_world, world_on_camera_ray_plane_z};

/// CPU-side setup for one projected shadow depth pass.
#[derive(Clone, Copy, Debug)]
pub struct ProjectedShadowLightSetup {
    pub light_view_proj: Mat4,
    /// Index in [`UiFrame::scene_lighting::punctual`].
    pub source_light_index: u32,
    /// Layer in the point shadow depth array.
    pub layer_index: u32,
}

/// Output of punctual shadow setup: sparse casters + lighting-index remap.
#[derive(Clone, Debug)]
pub struct PunctualShadowBuild {
    pub casters: Vec<ProjectedShadowLightSetup>,
    /// Lighting index → shadow layer, or `-1` when that light does not cast.
    pub light_index_to_layer: [i32; MAX_POINT_LIGHTS],
}

impl PunctualShadowBuild {
    pub fn empty() -> Self {
        Self {
            casters: Vec::new(),
            light_index_to_layer: [-1; MAX_POINT_LIGHTS],
        }
    }
}

/// Preferred view-up for a Z-up world (+Z vertical). Falls back to +Y when looking straight up/down.
#[inline]
pub fn z_up_shadow_view_up(forward: Vec3) -> Vec3 {
    let f = forward.normalize_or_zero();
    if f.z.abs() > 0.999 {
        Vec3::Y
    } else {
        Vec3::Z
    }
}

/// World-space half extent for shop/archive item inspect shadow frusta.
pub const INSPECT_SHADOW_HALF_XY: f32 = 52.0;

/// Box corners around a catalog inspect pivot for tight ortho fitting.
pub fn inspect_shadow_fit_corners(center: Vec3, half: f32) -> Vec<Vec3> {
    let h = half.max(8.0);
    vec![
        center + Vec3::new(-h, -h, -h),
        center + Vec3::new(h, -h, -h),
        center + Vec3::new(-h, h, -h),
        center + Vec3::new(h, h, -h),
        center + Vec3::new(-h, -h, h),
        center + Vec3::new(h, -h, h),
        center + Vec3::new(-h, h, h),
        center + Vec3::new(h, h, h),
    ]
}

/// Fallback ortho half-extent when room bounds are unavailable.
pub fn room_projected_shadow_half_xy(
    camera_h: f32,
    env_height_scale: f32,
    bounds_doc: Option<RoomEnvironmentBounds>,
) -> f32 {
    let window_half = camera_h * env_height_scale * 0.35;
    let Some(bounds) = bounds_doc else {
        return window_half.max(32.0);
    };
    let s = room_env_world_scale(camera_h, env_height_scale);
    let ext = bounds.max - bounds.min;
    let half_xy = ext.x.max(ext.y) * s * 0.5;
    window_half.max(half_xy).max(32.0)
}

fn shadow_fit_points(
    eye: Vec3,
    forward: Vec3,
    target: Vec3,
    corners_world: &[Vec3],
) -> Vec<Vec3> {
    let fwd = forward.normalize_or_zero();
    let mut fit_points: Vec<Vec3> = corners_world
        .iter()
        .copied()
        .filter(|&p| (p - eye).dot(fwd) > 0.25)
        .collect();
    if fit_points.len() < 4 {
        fit_points = corners_world.to_vec();
    }
    fit_points.push(target);
    fit_points
}

/// Fit ortho left/right/top/bottom in light view space so room corners stay inside the depth map.
fn fit_ortho_xy_rh(view: Mat4, fit_points: &[Vec3], fallback_half: f32) -> (f32, f32, f32, f32) {
    let h = fallback_half.max(1.0);
    if fit_points.is_empty() {
        return (-h, h, -h, h);
    }
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for &p in fit_points {
        let v = view.transform_point3(p);
        min_x = min_x.min(v.x);
        max_x = max_x.max(v.x);
        min_y = min_y.min(v.y);
        max_y = max_y.max(v.y);
    }
    let pad = h * 0.04 + 8.0;
    min_x -= pad;
    max_x += pad;
    min_y -= pad;
    max_y += pad;
    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;
    let hx = ((max_x - min_x) * 0.5).max(h);
    let hy = ((max_y - min_y) * 0.5).max(h);
    (cx - hx, cx + hx, cy - hy, cy + hy)
}

/// Fit near/far for an RH ortho shadow camera. Uses room corners plus the look-at
/// target so interior geometry is not clipped in front of the near plane.
fn fit_ortho_depth_rh(view: Mat4, fit_points: &[Vec3]) -> (f32, f32) {
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for &p in fit_points {
        let v = view.transform_point3(p);
        min_z = min_z.min(v.z);
        max_z = max_z.max(v.z);
    }
    let near = (-max_z).max(0.05);
    let far = (-min_z).max(near + 0.5);
    (near, far)
}

pub fn point_light_shadow_view_proj(
    light_world: Vec3,
    scene_corners_world: &[Vec3],
    fallback_half_xy: f32,
    fallback_depth: f32,
) -> Mat4 {
    point_light_shadow_view_proj_with_fit(
        light_world,
        scene_corners_world,
        fallback_half_xy,
        fallback_depth,
        Vec3::ZERO,
    )
    .0
}

pub fn point_light_shadow_view_proj_with_fit(
    light_world: Vec3,
    scene_corners_world: &[Vec3],
    fallback_half_xy: f32,
    fallback_depth: f32,
    look_at: Vec3,
) -> (Mat4, Option<(f32, f32, f32, f32)>) {
    let target = look_at;
    let eye = light_world;
    let forward = (target - eye).normalize_or_zero();
    if forward.length_squared() < 1e-10 {
        return (Mat4::IDENTITY, None);
    }
    let up = z_up_shadow_view_up(forward);
    let view = Mat4::look_at_rh(eye, target, up);
    let h = fallback_half_xy.max(1.0);
    let fit_points = shadow_fit_points(eye, forward, target, scene_corners_world);
    let (left, right, bottom, top) = fit_ortho_xy_rh(view, &fit_points, h);
    let (near, far) = if scene_corners_world.is_empty() {
        (0.05, fallback_depth.max(h * 2.0))
    } else {
        fit_ortho_depth_rh(view, &fit_points)
    };
    let proj = Mat4::orthographic_rh(left, right, bottom, top, near, far);
    (proj * view, Some((h, h, near, far)))
}

/// World-space punctual position — must stay in sync with [`crate::wgpu_renderer::lighting_buffers::PointLightsBuf`].
pub fn punctual_light_world(
    screen_w: f32,
    screen_h: f32,
    entry: &ScenePunctualLight,
    cam: Option<&CameraParams>,
    use_ray_plane: bool,
) -> Vec3 {
    match entry {
        ScenePunctualLight::Smooth(l) => {
            if use_ray_plane && let Some(cam) = cam {
                world_on_camera_ray_plane_z(screen_w, screen_h, cam, l.pos[0], l.pos[1], l.pos[2])
            } else {
                pixel_to_world(screen_w, screen_h, l.pos[0], l.pos[1], l.pos[2])
            }
        }
        ScenePunctualLight::InverseSquare(l) => {
            pixel_to_world(screen_w, screen_h, l.pos[0], l.pos[1], l.pos[2])
        }
    }
}

/// Build punctual shadow casters from scene policy (point lights only).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_punctual_shadow_setups(
    frame: &UiFrame,
    active_env: Option<ActiveRoomEnv>,
    screen_w: f32,
    screen_h: f32,
    camera_h: f32,
    env_height_scale: f32,
    bounds_doc: Option<RoomEnvironmentBounds>,
    cam: Option<&CameraParams>,
    use_ray_plane: bool,
    inspect_shadow_target: Option<[f32; 3]>,
) -> PunctualShadowBuild {
    let inspect_tight = inspect_shadow_target.is_some();
    let fallback_half = if inspect_tight {
        INSPECT_SHADOW_HALF_XY
    } else {
        room_projected_shadow_half_xy(camera_h, env_height_scale, bounds_doc)
    };
    let fallback_depth = if inspect_tight {
        INSPECT_SHADOW_HALF_XY * 2.5
    } else {
        bounds_doc
            .map(|b| {
                let s = room_env_world_scale(camera_h, env_height_scale);
                (b.max.z - b.min.z) * s * 1.25 + 64.0
            })
            .unwrap_or_else(|| camera_h * env_height_scale * 1.45)
    };
    let scene_corners = if let Some(target) = inspect_shadow_target {
        inspect_shadow_fit_corners(Vec3::from_array(target), INSPECT_SHADOW_HALF_XY)
    } else {
        bounds_doc
            .map(|b| room_world_bounds_corners_centered(b, camera_h, env_height_scale))
            .unwrap_or_default()
    };
    let look_at = inspect_shadow_target
        .map(Vec3::from_array)
        .unwrap_or(Vec3::ZERO);

    let mut out = PunctualShadowBuild::empty();
    let mut layer = 0u32;
    for (i, entry) in frame
        .scene_lighting
        .punctual
        .iter()
        .take(MAX_POINT_LIGHTS)
        .enumerate()
    {
        let cpu_node = active_env.and_then(|e| e.embedded_point_light_node_name(i));
        let node = frame
            .scene_lighting
            .punctual_gltf_node(i)
            .or(cpu_node.as_deref());
        if !punctual_light_casts_shadow(active_env, node) {
            continue;
        }
        let light_world = punctual_light_world(screen_w, screen_h, entry, cam, use_ray_plane);
        let light_view_proj = point_light_shadow_view_proj_with_fit(
            light_world,
            &scene_corners,
            fallback_half,
            fallback_depth,
            look_at,
        )
        .0;
        out.light_index_to_layer[i] = layer as i32;
        out.casters.push(ProjectedShadowLightSetup {
            light_view_proj,
            source_light_index: i as u32,
            layer_index: layer,
        });
        layer += 1;
    }
    out
}

#[inline]
fn projected_shadow_hash(build: &PunctualShadowBuild) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    build.casters.len().hash(&mut h);
    for light in &build.casters {
        light
            .light_view_proj
            .to_cols_array()
            .map(f32::to_bits)
            .hash(&mut h);
        light.source_light_index.hash(&mut h);
        light.layer_index.hash(&mut h);
    }
    build.light_index_to_layer.hash(&mut h);
    h.finish()
}

pub fn punctual_shadow_setups_changed(build: &PunctualShadowBuild, cached_hash: u64) -> (u64, bool) {
    let hash = projected_shadow_hash(build);
    (hash, hash != cached_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw_cmd::SceneLighting;
    use crate::wgpu_renderer::PointLight;
    use crate::world_space::surface_anchor_from_world_xyz;

    #[test]
    fn shadow_setup_uses_pixel_to_world_not_anchor_triple() {
        let w = 1920.0;
        let h = 1080.0;
        let world = Vec3::new(12.0, -4.0, 18.0);
        let anchor = surface_anchor_from_world_xyz(w, h, world);
        let mut frame = UiFrame::default();
        frame.scene_lighting = SceneLighting {
            punctual: vec![ScenePunctualLight::InverseSquare(PointLight {
                pos: anchor,
                radius: 40.0,
                color: [1.0, 0.8, 0.6],
                intensity: 2.0,
            })],
            ..Default::default()
        };
        let build = build_punctual_shadow_setups(
            &frame,
            Some(ActiveRoomEnv::Gameplay),
            w,
            h,
            h,
            1.0,
            None,
            None,
            false,
            None,
        );
        assert_eq!(build.casters.len(), 1);
        let clip = build.casters[0].light_view_proj * world.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(
            ndc.x.abs() <= 1.05 && ndc.y.abs() <= 1.05,
            "light at {world:?} should project near frustum center, got ndc {ndc:?}"
        );
    }

    #[test]
    fn shop_policy_skips_candles() {
        let mut frame = UiFrame::default();
        frame.scene_lighting.set_punctual_tagged([
            (
                ScenePunctualLight::InverseSquare(PointLight {
                    pos: [0.0; 3],
                    radius: 10.0,
                    color: [1.0; 3],
                    intensity: 1.0,
                }),
                Some("light_candle".to_string()),
            ),
            (
                ScenePunctualLight::InverseSquare(PointLight {
                    pos: [1.0; 3],
                    radius: 10.0,
                    color: [1.0; 3],
                    intensity: 1.0,
                }),
                Some("light_lantern".to_string()),
            ),
        ]);
        let build = build_punctual_shadow_setups(
            &frame,
            Some(ActiveRoomEnv::Shop),
            1920.0,
            1080.0,
            1080.0,
            1.0,
            None,
            None,
            false,
            None,
        );
        assert_eq!(build.casters.len(), 1);
        assert_eq!(build.casters[0].source_light_index, 1);
        assert_eq!(build.light_index_to_layer[0], -1);
        assert_eq!(build.light_index_to_layer[1], 0);
    }

    #[test]
    fn inspect_target_tight_frustum_centers_on_pivot() {
        let pivot = Vec3::new(40.0, -20.0, 36.0);
        let mut frame = UiFrame::default();
        frame.scene_lighting.set_punctual_tagged([(
            ScenePunctualLight::InverseSquare(PointLight {
                pos: [960.0, 400.0, 8000.0],
                radius: 200.0,
                color: [1.0; 3],
                intensity: 1.0,
            }),
            Some("light_lantern".to_string()),
        )]);
        let build = build_punctual_shadow_setups(
            &frame,
            Some(ActiveRoomEnv::Shop),
            1920.0,
            1080.0,
            1080.0,
            1.0,
            None,
            None,
            false,
            Some(pivot.to_array()),
        );
        assert_eq!(build.casters.len(), 1);
        let clip = build.casters[0].light_view_proj * pivot.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(
            ndc.x.abs() < 0.35 && ndc.y.abs() < 0.35,
            "inspect pivot should sit near shadow frustum center, got ndc {ndc:?}"
        );
    }

    #[test]
    fn fitted_frustum_covers_room_center_depth() {
        let bounds = RoomEnvironmentBounds {
            min: Vec3::new(-120.0, -80.0, 0.0),
            max: Vec3::new(120.0, 80.0, 160.0),
        };
        let h = 800.0;
        let env_h = 1.0;
        let corners = room_world_bounds_corners_centered(bounds, h, env_h);
        let max_corner_z = corners.iter().map(|c| c.z).fold(f32::NEG_INFINITY, f32::max);
        let light = Vec3::new(50.0, -30.0, max_corner_z + 5000.0);
        let fallback = room_projected_shadow_half_xy(h, env_h, Some(bounds));
        let (vp, fit) = point_light_shadow_view_proj_with_fit(light, &corners, fallback, 500.0, Vec3::ZERO);
        let (_, _, near, far) = fit.expect("fit stats");
        let view = {
            let target = Vec3::ZERO;
            let forward = (target - light).normalize();
            glam::Mat4::look_at_rh(light, target, z_up_shadow_view_up(forward))
        };
        let table_vz = view.transform_point3(Vec3::ZERO).z;
        assert!(
            table_vz <= -near && table_vz >= -far,
            "room center must lie inside depth range: table_vz={table_vz} near={near} far={far}"
        );
        let clip = vp * Vec3::ZERO.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(
            ndc.z >= 0.0 && ndc.z <= 1.0,
            "room center ndc.z should be in [0,1], got {ndc:?}"
        );
    }

    #[test]
    fn fitted_frustum_covers_room_corners_xy() {
        let bounds = RoomEnvironmentBounds {
            min: Vec3::new(-120.0, -80.0, 0.0),
            max: Vec3::new(120.0, 80.0, 160.0),
        };
        let h = 800.0;
        let env_h = 1.0;
        let corners = room_world_bounds_corners_centered(bounds, h, env_h);
        let max_corner_z = corners.iter().map(|c| c.z).fold(f32::NEG_INFINITY, f32::max);
        let light = Vec3::new(50.0, -30.0, max_corner_z + 5000.0);
        let fallback = room_projected_shadow_half_xy(h, env_h, Some(bounds));
        let (vp, _) = point_light_shadow_view_proj_with_fit(light, &corners, fallback, 500.0, Vec3::ZERO);
        for (i, &corner) in corners.iter().enumerate() {
            let clip = vp * corner.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.x >= -1.05 && ndc.x <= 1.05 && ndc.y >= -1.05 && ndc.y <= 1.05,
                "corner {i} should project inside shadow frustum, got ndc {ndc:?}"
            );
        }
    }
}
