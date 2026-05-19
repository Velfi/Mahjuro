//! Shared ray–triangle queries for picking and collision.

use glam::{Mat4, Vec3};

/// World-space ray intersection result.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    /// Distance along `world_dir` from `world_origin`.
    pub t: f32,
}

/// Möller–Trumbore against triangles in mesh-local space (`model` maps local → world).
pub fn ray_hit_trimesh(
    tris: &[[Vec3; 3]],
    model: Mat4,
    world_origin: Vec3,
    world_dir: Vec3,
) -> Option<RayHit> {
    let inv = model.inverse();
    let lo = inv.transform_point3(world_origin);
    let ld = inv.transform_vector3(world_dir);
    const EPS: f32 = 1e-7;
    let mut best: Option<f32> = None;
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
        match best {
            Some(bt) if bt <= wt => {}
            _ => best = Some(wt),
        }
    }
    best.map(|t| RayHit { t })
}
