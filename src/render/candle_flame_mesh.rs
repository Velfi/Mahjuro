//! Revolved teardrop mesh for Godot-style volumetric candle flames.
//!
//! Local space: **+Y** is up in mesh space (Godot convention); `flame.wgsl`
//! remaps to Mahjuro **+Z** in [`flame_volume`] / `flame.wgsl`. **y = 0** at the wick,
//! **y = 1** at the tip.
//! **uv.y = 0** at the wick, **uv.y = 1** at the tip (matches
//! [Godot lighter/candle flame](https://godotshaders.com/shader/lighter-candle-flame/)
//! `height = 1.0 - UV.y`).

use crate::render::lit_mesh::MeshCpu;
use crate::render::tile_glb::Vertex3dTex;

const RADIAL_SEGMENTS: usize = 20;
const HEIGHT_SEGMENTS: usize = 24;

fn smootherstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Radius at normalized height `t` ∈ [0, 1] (0 = wick, 1 = tip).
fn flame_radius_at(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    // Pinch the wick end to a point. A non-zero radius at t=0 creates a
    // visible capped disk, which reads as a flat flame bottom from low angles.
    let base = smootherstep(0.0, 0.22, t);
    let t = smootherstep(0.0, 1.0, t);
    let belly = (1.0 - (t - 0.22).powi(2) * 14.0).max(0.0);
    base * (0.36 * (1.0 - t).powf(0.48) + 0.13 * belly)
}

/// Build once at startup; instanced per candle in `flame.wgsl`.
pub fn build_candle_flame_volume_mesh() -> MeshCpu {
    let n_rad = RADIAL_SEGMENTS;
    let n_h = HEIGHT_SEGMENTS;
    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity((n_h + 1) * (n_rad + 1) + 2);
    let mut indices: Vec<u32> = Vec::with_capacity(n_h * n_rad * 6);

    for hi in 0..=n_h {
        let t = hi as f32 / n_h as f32;
        let y = t;
        let r = flame_radius_at(t);
        for ri in 0..=n_rad {
            let theta = (ri as f32) / (n_rad as f32) * std::f32::consts::TAU;
            let (sin_t, cos_t) = theta.sin_cos();
            let x = r * cos_t;
            let z = r * sin_t;
            let nx = cos_t;
            let nz = sin_t;
            let ny = 0.15 * (1.0 - t);
            let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            vertices.push(Vertex3dTex {
                position: [x, y, z],
                normal: [nx / len, ny / len, nz / len],
                uv: [ri as f32 / n_rad as f32, t],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
    }

    // Bottom cap is degenerate after the wick pinch; keeping the fan closes the
    // mesh without leaving a visible disk at the base.
    let wick_i = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, 0.0, 0.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.5, 0.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    for ri in 0..n_rad {
        let theta0 = (ri as f32) / (n_rad as f32) * std::f32::consts::TAU;
        let theta1 = ((ri + 1) as f32) / (n_rad as f32) * std::f32::consts::TAU;
        let (s0, c0) = theta0.sin_cos();
        let (s1, c1) = theta1.sin_cos();
        let n0 = [c0, -0.55, s0];
        let n1 = [c1, -0.55, s1];
        let len0 = (n0[0] * n0[0] + n0[1] * n0[1] + n0[2] * n0[2])
            .sqrt()
            .max(1e-6);
        let len1 = (n1[0] * n1[0] + n1[1] * n1[1] + n1[2] * n1[2])
            .sqrt()
            .max(1e-6);
        let ring0 = ri as u32;
        let ring1 = (ri + 1) as u32;
        vertices[ring0 as usize].normal = [n0[0] / len0, n0[1] / len0, n0[2] / len0];
        vertices[ring1 as usize].normal = [n1[0] / len1, n1[1] / len1, n1[2] / len1];
    }

    // Tip cap (degenerate radius — closes the teardrop).
    let tip_i = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, 1.0, 0.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.5, 1.0],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });

    let row = (n_rad + 1) as u32;
    for ri in 0..n_rad {
        indices.extend_from_slice(&[wick_i, ri as u32, (ri + 1) as u32]);
    }
    for hi in 0..n_h {
        for ri in 0..n_rad {
            let i0 = hi as u32 * row + ri as u32;
            let i1 = i0 + 1;
            let i2 = i0 + row;
            let i3 = i2 + 1;
            indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }
    let top_ring = (n_h - 1) as u32 * row;
    for ri in 0..n_rad {
        indices.extend_from_slice(&[top_ring + ri as u32, tip_i, top_ring + ri as u32 + 1]);
    }

    MeshCpu {
        vertices,
        indices,
        default_material: crate::render::lit_mesh::MaterialParams::wick(),
    }
}
