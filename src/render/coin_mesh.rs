//! Procedural mesh for a single gold coin (a flat 16-sided cylinder),
//! used by the shop scene to represent the player's gold as a physical
//! pile in a dish.
//!
//! The mesh is centered at local origin spanning -0.5..+0.5 on each axis
//! so per-instance scale can size and orient it. The flat top/bottom faces
//! get a single normal each (so they read as a clean disc under the lit
//! pipeline) and the side rim is a 16-quad ring.

use std::f32::consts::TAU;

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

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
    });
    let top_ring_start = vertices.len() as u32;
    for &(x, z) in ring.iter() {
        vertices.push(Vertex3dTex {
            position: [x, top_y, z],
            normal: top_normal,
            uv: [x + 0.5, z + 0.5],
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
    });
    let bot_ring_start = vertices.len() as u32;
    for &(x, z) in ring.iter() {
        vertices.push(Vertex3dTex {
            position: [x, bot_y, z],
            normal: bot_normal,
            uv: [x + 0.5, z + 0.5],
        });
    }
    for i in 0..SIDES {
        let i0 = bot_ring_start + i as u32;
        let i1 = bot_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[bot_center_idx, i1, i0]);
    }

    // ── Side rim: 16 quads with per-quad outward normal so the rim
    // catches highlights cleanly without smoothing across faces.
    for i in 0..SIDES {
        let (x0, z0) = ring[i];
        let (x1, z1) = ring[(i + 1) % SIDES];
        // Outward face normal = midpoint direction.
        let mx = (x0 + x1) * 0.5;
        let mz = (z0 + z1) * 0.5;
        let len = (mx * mx + mz * mz).sqrt().max(1e-6);
        let n = [mx / len, 0.0, mz / len];
        let base = vertices.len() as u32;
        // top0, top1, bot1, bot0
        vertices.push(Vertex3dTex {
            position: [x0, top_y, z0],
            normal: n,
            uv: [0.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, top_y, z1],
            normal: n,
            uv: [1.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, bot_y, z1],
            normal: n,
            uv: [1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x0, bot_y, z0],
            normal: n,
            uv: [0.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            // Bright polished gold; per-instance color overrides this.
            base_color: [1.00, 0.78, 0.30, 1.0],
            specular_strength: 0.7,
            specular_power: 64.0,
        },
    }
}
