//! CPU-side punctual lighting for screen-space effects (rain splashes, etc.).
//!
//! Mirrors the dielectric diffuse path in `room_glb.wgsl` / `scene_pbr_lights.wgsl`
//! closely enough that splashes pick up doorway warmth, moonlight cool fill, and
//! roof analytic occlusion.

use glam::Vec3;

use crate::draw_cmd::{CameraParams, ScenePunctualLight};
use crate::room_env_gltf::RoomCollisionMesh;
use crate::wgpu_renderer::{PointLight, SpotLight};
use crate::world_space::{pixel_to_world, world_on_camera_ray_plane_z};

const OCCLUSION_SHADOW_FLOOR: f32 = 0.14;
const DIELECTRIC_KD: f32 = 0.96;
const INV_PI: f32 = std::f32::consts::FRAC_1_PI;

/// World-space AABB used for analytic punctual occlusion (same test as `room_glb.wgsl`).
#[derive(Clone, Copy, Debug)]
pub struct PunctualOccluderAabb {
    pub center: Vec3,
    pub half_extents: Vec3,
}

impl PunctualOccluderAabb {
    /// Largest room collision volumes first, capped at [`crate::wgpu_renderer::MAX_TILE_OCCLUDERS`].
    pub fn from_room_collision_meshes(
        model: glam::Mat4,
        meshes: &[RoomCollisionMesh],
    ) -> Vec<Self> {
        let mut ranked: Vec<(f32, Self)> = Vec::new();
        for mesh in meshes {
            if mesh.node_name.starts_with("light_") {
                continue;
            }
            let (center, half) = room_collision_mesh_world_aabb(mesh, model);
            if !half.is_finite() || half.max_element() < 1e-5 {
                continue;
            }
            let volume = half.x * half.y * half.z;
            ranked.push((
                volume,
                Self {
                    center,
                    half_extents: half,
                },
            ));
        }
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        ranked
            .into_iter()
            .take(crate::wgpu_renderer::MAX_TILE_OCCLUDERS)
            .map(|(_, o)| o)
            .collect()
    }
}

#[inline]
fn room_collision_mesh_world_aabb(
    mesh: &RoomCollisionMesh,
    model: glam::Mat4,
) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for tri in &mesh.triangles {
        for p in tri {
            let w = model.transform_point3(*p);
            lo = lo.min(w);
            hi = hi.max(w);
        }
    }
    if !lo.is_finite() {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    let half = (hi - lo) * 0.5 * 1.03;
    let center = (lo + hi) * 0.5;
    (center, half)
}

#[derive(Clone, Copy, Debug)]
pub struct SceneLightSampleCtx<'a> {
    pub screen_w: f32,
    pub screen_h: f32,
    pub cam: Option<&'a CameraParams>,
    pub ambient_scale: f32,
    pub inv_doc_scale: f32,
    pub linear_exposure: f32,
    pub punctual: &'a [ScenePunctualLight],
    pub spots: &'a [SpotLight],
    pub occluders: &'a [PunctualOccluderAabb],
}

#[inline]
fn punctual_attenuation_khr(distance: f32, range_max: f32) -> f32 {
    let d = distance.max(1e-4);
    let mut att = 1.0 / (d * d);
    if range_max > 1e-5 {
        let x = (d / range_max).min(1.0);
        let window = (1.0 - x.powi(4)).max(0.0);
        att *= window;
    }
    att
}

#[inline]
fn punctual_attenuation_with_inv_doc_scale(
    dist_world: f32,
    range_world: f32,
    inv_doc_scale: f32,
) -> f32 {
    let d = if inv_doc_scale > 1e-8 {
        dist_world * inv_doc_scale
    } else {
        dist_world
    };
    let r = if inv_doc_scale > 1e-8 {
        range_world * inv_doc_scale
    } else {
        range_world
    };
    punctual_attenuation_khr(d, r)
}

#[inline]
fn scene_smooth_point_atten(dist: f32, radius: f32) -> f32 {
    let t = (1.0 - dist / radius.max(1.0)).clamp(0.0, 1.0);
    t * t
}

#[inline]
fn khr_spot_angle_attenuation(cos_a: f32, cos_inner: f32, cos_outer: f32) -> f32 {
    let den = (cos_inner - cos_outer).max(1e-3);
    let scale = 1.0 / den;
    let offset = -cos_outer * scale;
    let angular = (cos_a * scale + offset).clamp(0.0, 1.0);
    angular * angular
}

#[inline]
fn scene_segment_hits_aabb(
    light_pos: Vec3,
    inv_dir: Vec3,
    center: Vec3,
    half: Vec3,
    near_bias: f32,
    far_bias: f32,
) -> bool {
    let t1 = (center - half - light_pos) * inv_dir;
    let t2 = (center + half - light_pos) * inv_dir;
    let tmin = t1.min(t2);
    let tmax = t1.max(t2);
    let near_t = tmin.x.max(tmin.y).max(tmin.z);
    let far_t = tmax.x.min(tmax.y).min(tmax.z);
    far_t > near_t && near_t > near_bias && near_t < far_bias
}

fn punctual_occlusion(
    light_pos: Vec3,
    frag_pos: Vec3,
    occluders: &[PunctualOccluderAabb],
) -> f32 {
    if occluders.is_empty() {
        return 1.0;
    }
    let dir = frag_pos - light_pos;
    let dist = dir.length();
    if dist < 1e-4 {
        return 1.0;
    }
    let lp = light_pos;
    let ray = frag_pos - lp;
    let safe = ray + Vec3::splat(1e-6);
    let inv = Vec3::new(1.0 / safe.x, 1.0 / safe.y, 1.0 / safe.z);
    let near_bias = 0.015;
    let far_bias = 0.985;
    for occ in occluders {
        if scene_segment_hits_aabb(
            lp,
            inv,
            occ.center,
            occ.half_extents,
            near_bias,
            far_bias,
        ) {
            return 0.0;
        }
    }
    1.0
}

fn point_light_world(
    ctx: &SceneLightSampleCtx<'_>,
    light: &PointLight,
    inverse_square: bool,
) -> Vec3 {
    if inverse_square || ctx.cam.is_none() {
        pixel_to_world(
            ctx.screen_w,
            ctx.screen_h,
            light.pos[0],
            light.pos[1],
            light.pos[2],
        )
    } else if let Some(cam) = ctx.cam {
        world_on_camera_ray_plane_z(
            ctx.screen_w,
            ctx.screen_h,
            cam,
            light.pos[0],
            light.pos[1],
            light.pos[2],
        )
    } else {
        pixel_to_world(
            ctx.screen_w,
            ctx.screen_h,
            light.pos[0],
            light.pos[1],
            light.pos[2],
        )
    }
}

fn spot_light_world(ctx: &SceneLightSampleCtx<'_>, light: &SpotLight) -> (Vec3, Vec3) {
    let pos = if let Some(cam) = ctx.cam {
        world_on_camera_ray_plane_z(
            ctx.screen_w,
            ctx.screen_h,
            cam,
            light.pos[0],
            light.pos[1],
            light.pos[2],
        )
    } else {
        pixel_to_world(
            ctx.screen_w,
            ctx.screen_h,
            light.pos[0],
            light.pos[1],
            light.pos[2],
        )
    };
    let dir = Vec3::from(light.dir).normalize_or_zero();
    let dir = if dir.length_squared() < 0.5 {
        Vec3::new(0.0, 0.0, -1.0)
    } else {
        dir
    };
    (pos, dir)
}

fn punctual_visibility(
    ctx: &SceneLightSampleCtx<'_>,
    light_pos: Vec3,
    frag_pos: Vec3,
) -> f32 {
    if ctx.occluders.is_empty() {
        return 1.0;
    }
    let occ = punctual_occlusion(light_pos, frag_pos, ctx.occluders);
    OCCLUSION_SHADOW_FLOOR + (1.0 - OCCLUSION_SHADOW_FLOOR) * occ
}

fn finish_lit_rgb(
    base_rgb: [f32; 3],
    albedo: Vec3,
    lo: Vec3,
    ctx: &SceneLightSampleCtx<'_>,
) -> [f32; 3] {
    let ambient = albedo * ctx.ambient_scale.max(0.0);
    let hdr = lo * ctx.linear_exposure.max(0.0) + ambient;

    // Keep a small floor so unlit particles do not vanish; preserve authored hue.
    let floor = albedo * 0.08;
    let peak = hdr.max(floor);
    let scale = if albedo.max_element() > 1e-6 {
        peak / albedo
    } else {
        Vec3::splat(peak.max_element())
    };
    [
        (base_rgb[0] * scale.x).max(0.0),
        (base_rgb[1] * scale.y).max(0.0),
        (base_rgb[2] * scale.z).max(0.0),
    ]
}

fn accumulate_punctual_lo(
    world_pos: Vec3,
    albedo: Vec3,
    ctx: &SceneLightSampleCtx<'_>,
    ndl_at: impl Fn(Vec3) -> f32,
) -> Vec3 {
    let mut lo = Vec3::ZERO;

    for ent in ctx.punctual {
        let (light, inverse_square) = match ent {
            ScenePunctualLight::Smooth(l) => (l, false),
            ScenePunctualLight::InverseSquare(l) => (l, true),
        };
        let light_pos = point_light_world(ctx, light, inverse_square);
        let to_light = light_pos - world_pos;
        let dist = to_light.length();
        if dist < 1e-4 {
            continue;
        }
        let l_dir = to_light / dist;
        let atten = if inverse_square {
            punctual_attenuation_with_inv_doc_scale(dist, light.radius, ctx.inv_doc_scale)
        } else {
            scene_smooth_point_atten(dist, light.radius)
        };
        if atten <= 0.0 {
            continue;
        }
        let radiance = Vec3::from(light.color) * light.intensity.max(0.0) * atten;
        let ndl = ndl_at(l_dir);
        if ndl <= 0.0 || radiance.length_squared() <= 0.0 {
            continue;
        }
        let vis = punctual_visibility(ctx, light_pos, world_pos);
        lo += albedo * DIELECTRIC_KD * INV_PI * radiance * ndl * vis;
    }

    for spot in ctx.spots {
        let (light_pos, spot_dir) = spot_light_world(ctx, spot);
        let to_frag = world_pos - light_pos;
        let dist = to_frag.length();
        if dist < 1e-4 {
            continue;
        }
        let atten =
            punctual_attenuation_with_inv_doc_scale(dist, spot.radius, ctx.inv_doc_scale);
        if atten <= 0.0 {
            continue;
        }
        let frag_dir = to_frag / dist;
        let l_dir = -frag_dir;
        let cos_a = frag_dir.dot(spot_dir);
        let spot_factor = khr_spot_angle_attenuation(cos_a, spot.cos_inner, spot.cos_outer);
        if spot_factor <= 0.0 {
            continue;
        }
        let radiance = Vec3::from(spot.color)
            * spot.intensity.max(0.0)
            * atten
            * spot_factor;
        let ndl = ndl_at(l_dir);
        if ndl <= 0.0 {
            continue;
        }
        let vis = punctual_visibility(ctx, light_pos, world_pos);
        lo += albedo * DIELECTRIC_KD * INV_PI * radiance * ndl * vis;
    }

    lo
}

/// Dielectric RGB at a world-space surface point (rain splash albedo × scene lighting).
pub fn shade_dielectric_rgb_at_world(
    world_pos: Vec3,
    world_normal: Vec3,
    base_rgb: [f32; 3],
    ctx: &SceneLightSampleCtx<'_>,
) -> [f32; 3] {
    let n = world_normal.normalize_or_zero();
    let n = if n.length_squared() < 1e-6 {
        Vec3::Z
    } else {
        n
    };
    let albedo = Vec3::from(base_rgb);
    let lo = accumulate_punctual_lo(world_pos, albedo, ctx, |l_dir| n.dot(l_dir).max(0.0));
    finish_lit_rgb(base_rgb, albedo, lo, ctx)
}

/// Airborne rain streaks — punctual + ambient without a surface normal (soft wrap).
pub fn shade_volumetric_rgb_at_world(
    world_pos: Vec3,
    base_rgb: [f32; 3],
    ctx: &SceneLightSampleCtx<'_>,
) -> [f32; 3] {
    const VOLUMETRIC_NDL: f32 = 0.45;
    let albedo = Vec3::from(base_rgb);
    let lo = accumulate_punctual_lo(world_pos, albedo, ctx, |_| VOLUMETRIC_NDL);
    finish_lit_rgb(base_rgb, albedo, lo, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn khr_attenuation_matches_shader_window() {
        let att = punctual_attenuation_khr(10.0, 20.0);
        assert!(att > 0.0 && att < 0.02);
        assert_eq!(punctual_attenuation_khr(25.0, 20.0), 0.0);
    }
}
