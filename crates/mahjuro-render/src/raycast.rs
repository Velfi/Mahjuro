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

/// Median-split bounding-volume hierarchy over mesh-local triangles.
///
/// Built once (e.g. at glTF load) so per-frame ray-segment queries skip the
/// brute-force Möller–Trumbore over every triangle. Indices reference the
/// caller-owned `tris` slice passed to [`TriBvh::segment_hit`].
#[derive(Clone, Debug)]
pub struct TriBvh {
    nodes: Vec<BvhNode>,
    /// Triangle indices grouped by leaf (`order[start..start+count]`).
    order: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
struct BvhNode {
    min: Vec3,
    max: Vec3,
    /// Leaf (`count > 0`): `a` = start into `order`. Internal (`count == 0`): `a`/`b` = child node indices.
    a: u32,
    b: u32,
    count: u32,
}

/// Triangles per leaf; small enough that the leaf scan stays cheap.
const BVH_LEAF_MAX: usize = 4;

impl TriBvh {
    /// Build a BVH over `tris` (mesh-local space). Returns `None` for an empty soup.
    pub fn build(tris: &[[Vec3; 3]]) -> Option<Self> {
        if tris.is_empty() {
            return None;
        }
        let n = tris.len();
        let mut bounds: Vec<(Vec3, Vec3)> = Vec::with_capacity(n);
        let mut centroids: Vec<Vec3> = Vec::with_capacity(n);
        for t in tris {
            let mn = t[0].min(t[1]).min(t[2]);
            let mx = t[0].max(t[1]).max(t[2]);
            bounds.push((mn, mx));
            centroids.push((mn + mx) * 0.5);
        }
        let mut order: Vec<u32> = (0..n as u32).collect();
        let mut nodes: Vec<BvhNode> = Vec::with_capacity(2 * n);
        build_range(&mut nodes, &mut order, 0, n, &bounds, &centroids);
        Some(Self { nodes, order })
    }

    /// Triangle count covered by this BVH.
    pub fn tri_count(&self) -> usize {
        self.order.len()
    }

    /// Closest hit of the ray segment against `tris` (must be the same slice the BVH was built from).
    ///
    /// `local_origin`/`local_dir` are the world ray transformed by the mesh inverse
    /// (`local_dir = inv * world_dir_unit`), matching [`ray_segment_hits_local_aabb`]. The returned
    /// `RayHit::t` is in world units along `world_dir_unit` (the local param equals the world param
    /// when `local_dir = inv * world_dir_unit`). `max_t` bounds the segment in those world units.
    pub fn segment_hit(
        &self,
        tris: &[[Vec3; 3]],
        local_origin: Vec3,
        local_dir: Vec3,
        world_dir_unit: Vec3,
        model: Mat4,
        max_t: f32,
    ) -> Option<RayHit> {
        const EPS: f32 = 1e-7;
        let mut best: Option<(f32, Vec3)> = None;
        let mut best_t = max_t;
        // DFS stack of node indices. Depth is bounded by tree height (~log2(n)); 64 is ample.
        let mut stack = [0u32; 64];
        let mut sp = 0usize;
        stack[sp] = 0;
        sp += 1;
        while sp > 0 {
            sp -= 1;
            let node = self.nodes[stack[sp] as usize];
            if !ray_segment_hits_local_aabb(local_origin, local_dir, node.min, node.max, best_t) {
                continue;
            }
            if node.count == 0 {
                if sp + 2 <= stack.len() {
                    stack[sp] = node.a;
                    sp += 1;
                    stack[sp] = node.b;
                    sp += 1;
                }
                continue;
            }
            for k in node.a..node.a + node.count {
                let [a, b, c] = tris[self.order[k as usize] as usize];
                let e1 = b - a;
                let e2 = c - a;
                let p = local_dir.cross(e2);
                let det = e1.dot(p);
                if det.abs() < EPS {
                    continue;
                }
                let inv_det = 1.0 / det;
                let s = local_origin - a;
                let u = s.dot(p) * inv_det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = s.cross(e1);
                let v = local_dir.dot(q) * inv_det;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let t = e2.dot(q) * inv_det;
                if t <= EPS || t >= best_t {
                    continue;
                }
                let mut n_world = model.transform_vector3(e1.cross(e2)).normalize_or_zero();
                if n_world.dot(world_dir_unit) > 0.0 {
                    n_world = -n_world;
                }
                best_t = t;
                best = Some((t, n_world));
            }
        }
        best.map(|(t, normal)| RayHit { t, normal })
    }
}

fn build_range(
    nodes: &mut Vec<BvhNode>,
    order: &mut Vec<u32>,
    start: usize,
    end: usize,
    bounds: &[(Vec3, Vec3)],
    centroids: &[Vec3],
) -> u32 {
    let mut mn = Vec3::splat(f32::INFINITY);
    let mut mx = Vec3::splat(f32::NEG_INFINITY);
    for &i in &order[start..end] {
        let (bmn, bmx) = bounds[i as usize];
        mn = mn.min(bmn);
        mx = mx.max(bmx);
    }
    let node_idx = nodes.len() as u32;
    let count = end - start;
    nodes.push(BvhNode {
        min: mn,
        max: mx,
        a: start as u32,
        b: 0,
        count: count as u32,
    });

    if count <= BVH_LEAF_MAX {
        return node_idx;
    }

    let mut cmn = Vec3::splat(f32::INFINITY);
    let mut cmx = Vec3::splat(f32::NEG_INFINITY);
    for &i in &order[start..end] {
        let c = centroids[i as usize];
        cmn = cmn.min(c);
        cmx = cmx.max(c);
    }
    let ext = cmx - cmn;
    let axis = if ext.x >= ext.y && ext.x >= ext.z {
        0
    } else if ext.y >= ext.z {
        1
    } else {
        2
    };
    if ext[axis] < 1e-6 {
        return node_idx; // degenerate centroid spread: keep as leaf
    }

    let split = (cmn[axis] + cmx[axis]) * 0.5;
    let mut mid = start;
    for j in start..end {
        if centroids[order[j] as usize][axis] < split {
            order.swap(mid, j);
            mid += 1;
        }
    }
    if mid == start || mid == end {
        mid = start + count / 2; // fall back to median-by-count
    }

    let left = build_range(nodes, order, start, mid, bounds, centroids);
    let right = build_range(nodes, order, mid, end, bounds, centroids);
    let node = &mut nodes[node_idx as usize];
    node.a = left;
    node.b = right;
    node.count = 0;
    node_idx
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

    fn grid_mesh(n: usize) -> Vec<[Vec3; 3]> {
        let mut tris = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let (x, y) = (i as f32, j as f32);
                let p = |dx: f32, dy: f32| Vec3::new(x + dx, y + dy, 0.0);
                tris.push([p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)]);
                tris.push([p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)]);
            }
        }
        tris
    }

    #[test]
    fn bvh_matches_brute_force_on_grid() {
        let tris = grid_mesh(20);
        let bvh = TriBvh::build(&tris).expect("non-empty");
        assert_eq!(bvh.tri_count(), tris.len());
        let model = Mat4::IDENTITY;
        for (ox, oy) in [(2.3_f32, 5.7_f32), (10.5, 10.5), (0.1, 19.9), (-5.0, 5.0)] {
            let origin = Vec3::new(ox, oy, 10.0);
            let dir = Vec3::new(0.0, 0.0, -1.0);
            let max_t = 100.0;
            let brute = ray_hit_trimesh(&tris, model, origin, dir)
                .filter(|h| h.t > 1e-5 && h.t <= max_t);
            let fast = bvh.segment_hit(&tris, origin, dir, dir, model, max_t);
            match (brute, fast) {
                (Some(b), Some(f)) => {
                    assert!((b.t - f.t).abs() < 1e-3, "t mismatch {} vs {}", b.t, f.t);
                }
                (None, None) => {}
                (b, f) => panic!("hit/miss disagreement: brute={b:?} fast={f:?}"),
            }
        }
    }
}
