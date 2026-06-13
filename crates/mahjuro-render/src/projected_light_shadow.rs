//! Xbox-style projected depth maps: one orthographic view per punctual shadow
//! caster, aimed at the room center. Punctual-only; spot shadows are unsupported.
//!
//! World space is **Z-up** (table in XY, +Z up). Light positions must match
//! the GPU punctual buffer — see [`punctual_light_world`].

use glam::{Mat4, Vec3};

use crate::draw_cmd::{CameraParams, ScenePunctualLight, UiFrame};
use crate::room_env_gltf::{RoomEnvironmentBounds, room_world_bounds_corners_centered};
use crate::room_glb::{
    player_consumable_marker_name, player_relic_marker_name, room_env_world_scale,
    spawn_relic_marker_name, with_shop_glb_cpu,
};
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
    if f.z.abs() > 0.999 { Vec3::Y } else { Vec3::Z }
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

fn shadow_fit_points(eye: Vec3, forward: Vec3, target: Vec3, corners_world: &[Vec3]) -> Vec<Vec3> {
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

struct ShadowFitRegion {
    corners: Vec<Vec3>,
    look_at: Vec3,
    fallback_half: f32,
    fallback_depth: f32,
}

fn aabb_corners(min: Vec3, max: Vec3) -> Vec<Vec3> {
    vec![
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ]
}

fn shop_dynamic_shadow_fit_region(camera_h: f32, env_height_scale: f32) -> Option<ShadowFitRegion> {
    let points = with_shop_glb_cpu(|cpu_opt| {
        let cpu = cpu_opt?;
        let mut points = Vec::new();
        for slot in 0..9 {
            if let Some(p) =
                crate::room_glb::marker_translation(cpu, &spawn_relic_marker_name(slot))
            {
                points.push(p);
            }
        }
        for slot in 0..5 {
            if let Some(p) =
                crate::room_glb::marker_translation(cpu, &player_relic_marker_name(slot))
            {
                points.push(p);
            }
        }
        for slot in 0..2 {
            if let Some(p) =
                crate::room_glb::marker_translation(cpu, &player_consumable_marker_name(slot))
            {
                points.push(p);
            }
        }
        Some(points)
    })?;
    if points.is_empty() {
        return None;
    }
    let scale = room_env_world_scale(camera_h, env_height_scale);
    let mut min_v = Vec3::splat(f32::INFINITY);
    let mut max_v = Vec3::splat(f32::NEG_INFINITY);
    for p in points {
        let w = p * scale;
        min_v = min_v.min(w);
        max_v = max_v.max(w);
    }
    let pad_xy = camera_h * 0.22;
    let pad_front_back = camera_h * 0.34;
    let pad_down = camera_h * 0.18;
    let pad_up = camera_h * 0.62;
    min_v -= Vec3::new(pad_xy, pad_front_back, pad_down);
    max_v += Vec3::new(pad_xy, pad_front_back, pad_up);

    let ext = max_v - min_v;
    let fallback_half = (ext.x.max(ext.y).max(ext.z) * 0.55).max(camera_h * 0.20);
    let fallback_depth = ext.length().max(camera_h * 0.75);
    Some(ShadowFitRegion {
        corners: aabb_corners(min_v, max_v),
        look_at: (min_v + max_v) * 0.5,
        fallback_half,
        fallback_depth,
    })
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
        ScenePunctualLight::Smooth(l) | ScenePunctualLight::SmoothNoShadow(l) => {
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

/// Build punctual shadow casters from scene punctual lights (point lights only).
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
    focus_shop_dynamic_region: bool,
) -> PunctualShadowBuild {
    let lab_layout = frame.shadow_ao_lab_layout;
    let shop_dynamic_fit = if focus_shop_dynamic_region && active_env == Some(ActiveRoomEnv::Shop) {
        shop_dynamic_shadow_fit_region(camera_h, env_height_scale)
    } else {
        None
    };
    let fallback_half = if let Some(fit) = &shop_dynamic_fit {
        fit.fallback_half
    } else if lab_layout.is_some() {
        crate::shadow_ao_lab::PUNCTUAL_SHADOW_FALLBACK_HALF
    } else {
        room_projected_shadow_half_xy(camera_h, env_height_scale, bounds_doc)
    };
    let fallback_depth = if let Some(fit) = &shop_dynamic_fit {
        fit.fallback_depth
    } else if lab_layout.is_some() {
        crate::shadow_ao_lab::PUNCTUAL_SHADOW_FALLBACK_DEPTH
    } else {
        bounds_doc
            .map(|b| {
                let s = room_env_world_scale(camera_h, env_height_scale);
                (b.max.z - b.min.z) * s * 1.25 + 64.0
            })
            .unwrap_or_else(|| camera_h * env_height_scale * 1.45)
    };
    let scene_corners = if let Some(fit) = &shop_dynamic_fit {
        fit.corners.clone()
    } else if let Some(layout) = lab_layout {
        crate::shadow_ao_lab::fit_corners_world(layout)
    } else {
        bounds_doc
            .map(|b| room_world_bounds_corners_centered(b, camera_h, env_height_scale))
            .unwrap_or_default()
    };
    let look_at = if let Some(fit) = &shop_dynamic_fit {
        fit.look_at
    } else if lab_layout.is_some() {
        crate::shadow_ao_lab::punctual_shadow_look_at()
    } else {
        Vec3::ZERO
    };

    let mut out = PunctualShadowBuild::empty();
    let mut layer = 0u32;
    for (i, entry) in frame
        .scene_lighting
        .punctual
        .iter()
        .take(MAX_POINT_LIGHTS)
        .enumerate()
    {
        if !entry.casts_shadow() {
            out.light_index_to_layer[i] = -1;
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

pub fn punctual_shadow_setups_changed(
    build: &PunctualShadowBuild,
    cached_hash: u64,
) -> (u64, bool) {
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
            false,
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
    fn shop_embedded_punctuals_cast() {
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
            false,
        );
        assert_eq!(build.casters.len(), 2);
        assert_eq!(build.casters[0].source_light_index, 0);
        assert_eq!(build.casters[1].source_light_index, 1);
        assert_eq!(build.light_index_to_layer[0], 0);
        assert_eq!(build.light_index_to_layer[1], 1);
    }

    #[test]
    fn no_shadow_punctual_keeps_light_index_without_shadow_layer() {
        let mut frame = UiFrame::default();
        frame.scene_lighting.punctual = vec![
            ScenePunctualLight::Smooth(PointLight {
                pos: [0.0; 3],
                radius: 10.0,
                color: [1.0; 3],
                intensity: 1.0,
            }),
            ScenePunctualLight::SmoothNoShadow(PointLight {
                pos: [1.0; 3],
                radius: 10.0,
                color: [1.0; 3],
                intensity: 1.0,
            }),
            ScenePunctualLight::InverseSquare(PointLight {
                pos: [2.0; 3],
                radius: 10.0,
                color: [1.0; 3],
                intensity: 1.0,
            }),
        ];
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
            false,
        );
        assert_eq!(build.casters.len(), 2);
        assert_eq!(build.casters[0].source_light_index, 0);
        assert_eq!(build.casters[0].layer_index, 0);
        assert_eq!(build.casters[1].source_light_index, 2);
        assert_eq!(build.casters[1].layer_index, 1);
        assert_eq!(build.light_index_to_layer[0], 0);
        assert_eq!(build.light_index_to_layer[1], -1);
        assert_eq!(build.light_index_to_layer[2], 1);
    }

    #[test]
    fn shop_dynamic_shadow_fit_is_tighter_than_whole_room() {
        let h = 1080.0;
        let env_h = 1.0;
        let bounds =
            crate::room_glb::with_shop_glb_cpu(|o| o.and_then(|c| c.environment_bounds_doc))
                .expect("shop bounds");
        let room_half = room_projected_shadow_half_xy(h, env_h, Some(bounds));
        let fit = shop_dynamic_shadow_fit_region(h, env_h).expect("shop stock fit");

        assert!(
            fit.fallback_half < room_half * 0.65,
            "shop stock shadow fit should be narrower than whole-room fit: stock={} room={}",
            fit.fallback_half,
            room_half
        );
        assert_eq!(fit.corners.len(), 8);
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
        let max_corner_z = corners
            .iter()
            .map(|c| c.z)
            .fold(f32::NEG_INFINITY, f32::max);
        let light = Vec3::new(50.0, -30.0, max_corner_z + 5000.0);
        let fallback = room_projected_shadow_half_xy(h, env_h, Some(bounds));
        let (vp, fit) =
            point_light_shadow_view_proj_with_fit(light, &corners, fallback, 500.0, Vec3::ZERO);
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
    fn lab_live_punctual_lvp_matches_synthetic_bake() {
        use crate::draw_cmd::{SceneLighting, ScenePunctualLight, UiFrame};
        use crate::shadow_ao_lab::{ShadowAoLabLayout, punctual_light_view_proj};
        use crate::wgpu_renderer::PointLight;

        let w = 1920.0;
        let h = 1080.0;
        let mut frame = UiFrame::default();
        frame.shadow_ao_lab_layout = Some(ShadowAoLabLayout::HorizontalBand);
        frame.scene_lighting = SceneLighting {
            punctual: vec![ScenePunctualLight::Smooth(PointLight {
                pos: crate::world_space::surface_anchor_from_world_xyz(
                    w,
                    h,
                    crate::shadow_ao_lab::light_world(),
                ),
                radius: h * 3.5,
                color: [1.0, 0.92, 0.78],
                intensity: 6.0,
            })],
            ..Default::default()
        };
        let build =
            build_punctual_shadow_setups(&frame, None, w, h, h, 1.0, None, None, false, false);
        assert_eq!(
            build.casters.len(),
            1,
            "lab should produce one shadow caster"
        );
        let expected = punctual_light_view_proj(ShadowAoLabLayout::HorizontalBand);
        let got = build.casters[0].light_view_proj.to_cols_array();
        let exp = expected.to_cols_array();
        for (i, (&g, &e)) in got.iter().zip(exp.iter()).enumerate() {
            assert!((g - e).abs() < 1e-4, "LVP[{i}] live={g} synthetic={e}");
        }
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
        let max_corner_z = corners
            .iter()
            .map(|c| c.z)
            .fold(f32::NEG_INFINITY, f32::max);
        let light = Vec3::new(50.0, -30.0, max_corner_z + 5000.0);
        let fallback = room_projected_shadow_half_xy(h, env_h, Some(bounds));
        let (vp, _) =
            point_light_shadow_view_proj_with_fit(light, &corners, fallback, 500.0, Vec3::ZERO);
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
