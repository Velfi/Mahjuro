//! Procedural mesh for a single gold coin (a flat 16-sided cylinder),
//! used by the shop scene to represent the player's gold as a physical
//! pile in a dish.
//!
//! The mesh is centered at local origin spanning -0.5..+0.5 on each axis
//! so per-instance scale can size and orient it. The flat top/bottom faces
//! get a single normal each (so they read as a clean disc under the lit
//! pipeline) and the side rim is a 16-quad ring.

use std::f32::consts::TAU;

use crate::cap_extrude::planar_y_cap_uv_xz;
use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::theme::color;
use crate::tile_glb::Vertex3dTex;

const SIDES: usize = 16;

/// Build a unit gold coin mesh: radius 0.5 in X/Z, half-thickness 0.5 in Y.
/// Per-instance scale shrinks the Y to coin thickness.
pub fn build_coin_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Generate ring positions (radius 0.5).
    let ring: Vec<(f32, f32)> = (0..SIDES)
        .map(|i| {
            let theta = (i as f32) * TAU / SIDES as f32;
            (0.5 * theta.cos(), 0.5 * theta.sin())
        })
        .collect();

    // ── Top face (+Y normal): triangle fan from center vertex.
    let top_y = 0.5;
    let top_normal = [0.0, 1.0, 0.0];
    let top_center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, top_y, 0.0],
        normal: top_normal,
        uv: [0.5, 0.5],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let top_ring_start = vertices.len() as u32;
    for &(x, z) in ring.iter() {
        vertices.push(Vertex3dTex {
            position: [x, top_y, z],
            normal: top_normal,
            uv: planar_y_cap_uv_xz(x, z),
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..SIDES {
        let i0 = top_ring_start + i as u32;
        let i1 = top_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[top_center_idx, i0, i1]);
    }

    // ── Bottom face (-Y normal): triangle fan, opposite winding.
    let bot_y = -0.5;
    let bot_normal = [0.0, -1.0, 0.0];
    let bot_center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, bot_y, 0.0],
        normal: bot_normal,
        uv: [0.5, 0.5],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let bot_ring_start = vertices.len() as u32;
    for &(x, z) in ring.iter() {
        vertices.push(Vertex3dTex {
            position: [x, bot_y, z],
            normal: bot_normal,
            uv: planar_y_cap_uv_xz(x, z),
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..SIDES {
        let i0 = bot_ring_start + i as u32;
        let i1 = bot_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[bot_center_idx, i1, i0]);
    }

    // ── Side rim: shared ring of vertices with radial per-vertex normals,
    // so the rim shades as a smooth curved edge instead of 16 facets.
    let rim_top_start = vertices.len() as u32;
    for (i, &(x, z)) in ring.iter().enumerate() {
        let len = (x * x + z * z).sqrt().max(1e-6);
        let n = [x / len, 0.0, z / len];
        let u = i as f32 / SIDES as f32;
        vertices.push(Vertex3dTex {
            position: [x, top_y, z],
            normal: n,
            uv: [u, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    let rim_bot_start = vertices.len() as u32;
    for (i, &(x, z)) in ring.iter().enumerate() {
        let len = (x * x + z * z).sqrt().max(1e-6);
        let n = [x / len, 0.0, z / len];
        let u = i as f32 / SIDES as f32;
        vertices.push(Vertex3dTex {
            position: [x, bot_y, z],
            normal: n,
            uv: [u, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..SIDES {
        let j = ((i + 1) % SIDES) as u32;
        let i = i as u32;
        let t0 = rim_top_start + i;
        let t1 = rim_top_start + j;
        let b0 = rim_bot_start + i;
        let b1 = rim_bot_start + j;
        indices.extend_from_slice(&[t0, t1, b1, t0, b1, b0]);
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Metal,
            // Bright polished gold; per-instance color overrides this and
            // also tints the metallic specular lobe in the shader.
            base_color: color::RELIC_GOLD,
            specular_strength: 1.0,
            specular_power: 96.0,
        },
    }
}
