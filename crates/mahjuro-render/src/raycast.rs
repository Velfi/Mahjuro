//! Shared ray–triangle queries for picking and collision.

use glam::{Mat4, Vec3};

/// World-space ray intersection result.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    /// Distance along `world_dir` from `world_origin`.
    pub t: f32,
    /// Shaded face normal in world space (points against the incoming ray).
    pub normal: Vec3,
}

/// Slab test: true when the ray segment `[0, max_t]` intersects `min`/`max` in the
/// same parameterization as `local_origin + local_dir * t`.
pub fn ray_segment_hits_local_aabb(
    local_origin: Vec3,
    local_dir: Vec3,
    min: Vec3,
    max: Vec3,
    max_t: f32,
) -> bool {
    const EPS: f32 = 1e-7;
    let mut t_enter = 0.0_f32;
    let mut t_exit = max_t;
    for axis in 0..3 {
        let (lo_a, ld_a, min_a, max_a) = match axis {
            0 => (local_origin.x, local_dir.x, min.x, max.x),
            1 => (local_origin.y, local_dir.y, min.y, max.y),
            _ => (local_origin.z, local_dir.z, min.z, max.z),
        };
        if ld_a.abs() < EPS {
            if lo_a < min_a || lo_a > max_a {
                return false;
            }
            continue;
        }
        let inv = 1.0 / ld_a;
        let mut t0 = (min_a - lo_a) * inv;
        let mut t1 = (max_a - lo_a) * inv;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        t_enter = t_enter.max(t0);
        t_exit = t_exit.min(t1);
        if t_enter > t_exit {
            return false;
        }
    }
    t_exit >= 0.0 && t_enter <= max_t
}

/// Möller–Trumbore against triangles in mesh-local space (`model` maps local → world).
pub fn ray_hit_trimesh(
    tris: &[[Vec3; 3]],
    model: Mat4,
    world_origin: Vec3,
    world_dir: Vec3,
) -> Option<RayHit> {
    ray_hit_trimesh_inv(tris, model, model.inverse(), world_origin, world_dir)
}

/// Like [`ray_hit_trimesh`] but reuses a precomputed local→world inverse.
pub fn ray_hit_trimesh_inv(
    tris: &[[Vec3; 3]],
    model: Mat4,
    inv: Mat4,
    world_origin: Vec3,
    world_dir: Vec3,
) -> Option<RayHit> {
    let lo = inv.transform_point3(world_origin);
    let ld = inv.transform_vector3(world_dir);
    let world_dir_unit = world_dir.normalize_or_zero();
    const EPS: f32 = 1e-7;
    let mut best: Option<(f32, Vec3)> = None;
    for [a, b, c] in tris {
        let e1 = *b - *a;
        let e2 = *c - *a;
        let p = ld.cross(e2);
        let det = e1.dot(p);
        if det.abs() < EPS {
            continue;
        }
        let inv_det = 1.0 / det;
        let s = lo - *a;
        let u = s.dot(p) * inv_det;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = ld.dot(q) * inv_det;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let t_loc = e2.dot(q) * inv_det;
        if t_loc <= EPS {
            continue;
        }
        let local_hit = lo + ld * t_loc;
        let world_hit = model.transform_point3(local_hit);
        let wt = (world_hit - world_origin).dot(world_dir);
        if wt <= EPS {
            continue;
        }
        let mut n_world = model.transform_vector3(e1.cross(e2)).normalize_or_zero();
        if n_world.dot(world_dir_unit) > 0.0 {
            n_world = -n_world;
        }
        match best {
            Some((bt, _)) if bt <= wt => {}
            _ => best = Some((wt, n_world)),
        }
    }
    best.map(|(t, normal)| RayHit { t, normal })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn local_aabb_accepts_segment_inside() {
        let min = Vec3::new(0.0, 0.0, 0.0);
        let max = Vec3::new(10.0, 10.0, 10.0);
        assert!(ray_segment_hits_local_aabb(
            Vec3::new(5.0, 5.0, 15.0),
            Vec3::new(0.0, 0.0, -1.0),
            min,
            max,
            20.0,
        ));
    }

    #[test]
    fn local_aabb_rejects_segment_before_box() {
        let min = Vec3::new(0.0, 0.0, 0.0);
        let max = Vec3::new(10.0, 10.0, 10.0);
        assert!(!ray_segment_hits_local_aabb(
            Vec3::new(5.0, 5.0, 20.0),
            Vec3::new(0.0, 0.0, -1.0),
            min,
            max,
            5.0,
        ));
    }
}
