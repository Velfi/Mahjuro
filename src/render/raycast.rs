//! Shared ray–triangle queries for picking and rain collision.

use glam::{Mat4, Vec3};

use crate::render::room_env_gltf::RoomCollisionMesh;

/// World-space ray intersection result.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    /// Distance along `world_dir` from `world_origin`.
    pub t: f32,
    pub point: Vec3,
    pub _normal: Vec3,
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
    let mut best: Option<(f32, Vec3, Vec3)> = None;
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
        let normal = e1.cross(e2).normalize_or_zero();
        let world_normal = model.transform_vector3(normal).normalize_or_zero();
        match best {
            Some((bt, _, _)) if bt <= wt => {}
            _ => best = Some((wt, world_hit, world_normal)),
        }
    }
        best.map(|(t, point, normal)| RayHit {
            t,
            point,
            _normal: normal,
        })
}

/// Closest hit along `world_dir` across decoded room collision meshes (same basis as picking).
pub fn ray_hit_trimesh_meshes(
    world_origin: Vec3,
    world_dir: Vec3,
    model: Mat4,
    meshes: &[RoomCollisionMesh],
) -> Option<RayHit> {
    let mut best: Option<RayHit> = None;
    for mesh in meshes {
        if let Some(hit) = ray_hit_trimesh(&mesh.triangles, model, world_origin, world_dir) {
            best = Some(match best {
                Some(b) if b.t <= hit.t => b,
                _ => hit,
            });
        }
    }
    best
}

/// Segment cast: returns the first hit with `t` in `(0, max_t]`.
#[inline]
pub fn ray_segment_hit_trimesh_meshes(
    world_origin: Vec3,
    world_dir: Vec3,
    max_t: f32,
    model: Mat4,
    meshes: &[RoomCollisionMesh],
) -> Option<RayHit> {
    ray_hit_trimesh_meshes(world_origin, world_dir, model, meshes).and_then(|h| {
        if h.t > max_t {
            None
        } else {
            Some(h)
        }
    })
}
