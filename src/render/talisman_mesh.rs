//! Procedural mesh for a hanging talisman (jade tablet) used by the shop.
//!
//! The mesh is a flat octagonal prism: an 8-sided "coin" oriented so its
//! flat faces point along ±Z and its long axis along Y. Local extents fit
//! inside the unit cube [-0.5, 0.5]^3 so a per-instance scale matrix can
//! turn it into varied tablet sizes. Each face has its own normal so the
//! lit-mesh shader reads it as flat-shaded jade.
//!
//! Layout:
//! - Front face (+Z normal) — octagon, slightly inset corners
//! - Back face  (-Z normal) — same octagon, opposite winding
//! - 8 rim quads connecting front and back, each with its own outward normal
//!
//! Width × height × thickness ≈ 1.0 × 1.4 × 0.18 in local units (the prism is
//! taller than wide, like a real hanging jade tablet). The scene scales it
//! up to whatever world-space size the talisman should display at.

use std::f32::consts::TAU;

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

const SIDES: usize = 8;
const HALF_W: f32 = 0.50;
const HALF_H: f32 = 0.70;
const HALF_T: f32 = 0.09;

/// Build the talisman mesh (octagonal flat tablet, hangs portrait-orientation).
pub fn build_talisman_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Compute the 8 rim corner positions in local XY space. Octagonal, with
    // slightly stretched Y so the tablet reads as taller than wide.
    let rim: Vec<(f32, f32)> = (0..SIDES)
        .map(|i| {
            // Start at the top center and go clockwise.
            let theta = (i as f32) * TAU / SIDES as f32 - TAU * 0.25;
            let cx = theta.cos();
            let cy = theta.sin();
            // Stretch Y to make the tablet taller than wide.
            (cx * HALF_W, -cy * HALF_H)
        })
        .collect();

    // ── Front face (+Z normal): triangle fan from center vertex.
    let front_z = HALF_T;
    let front_normal = [0.0, 0.0, 1.0];
    let front_center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, 0.0, front_z],
        normal: front_normal,
        uv: [0.5, 0.5],
    });
    let front_ring_start = vertices.len() as u32;
    for &(x, y) in rim.iter() {
        vertices.push(Vertex3dTex {
            position: [x, y, front_z],
            normal: front_normal,
            uv: [x / HALF_W * 0.5 + 0.5, 0.5 + y / HALF_H * 0.5],
        });
    }
    for i in 0..SIDES {
        let i0 = front_ring_start + i as u32;
        let i1 = front_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[front_center_idx, i0, i1]);
    }

    // ── Back face (-Z normal): triangle fan, opposite winding.
    let back_z = -HALF_T;
    let back_normal = [0.0, 0.0, -1.0];
    let back_center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, 0.0, back_z],
        normal: back_normal,
        uv: [0.5, 0.5],
    });
    let back_ring_start = vertices.len() as u32;
    for &(x, y) in rim.iter() {
        vertices.push(Vertex3dTex {
            position: [x, y, back_z],
            normal: back_normal,
            uv: [x / HALF_W * 0.5 + 0.5, 0.5 + y / HALF_H * 0.5],
        });
    }
    for i in 0..SIDES {
        let i0 = back_ring_start + i as u32;
        let i1 = back_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[back_center_idx, i1, i0]);
    }

    // ── Rim: 8 quads, each with its own outward normal.
    for i in 0..SIDES {
        let (x0, y0) = rim[i];
        let (x1, y1) = rim[(i + 1) % SIDES];
        // Outward normal = midpoint direction in XY.
        let mx = (x0 + x1) * 0.5;
        let my = (y0 + y1) * 0.5;
        let len = (mx * mx + my * my).sqrt().max(1e-6);
        let n = [mx / len, my / len, 0.0];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [x0, y0, front_z],
            normal: n,
            uv: [0.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, y1, front_z],
            normal: n,
            uv: [1.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, y1, back_z],
            normal: n,
            uv: [1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x0, y0, back_z],
            normal: n,
            uv: [0.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Jade,
            // Default jade-green; per-instance color overrides this.
            base_color: [0.45, 0.78, 0.55, 1.0],
            specular_strength: 0.55,
            specular_power: 48.0,
        },
    }
}

/// Local AABB half-extents for `pick_shop_object`'s slab test. Matches the
/// vertex layout above.
pub const TALISMAN_LOCAL_HALF: [f32; 3] = [HALF_W, HALF_H, HALF_T];

/// Per-talisman material parameters. Each variant gets its own `MaterialKind`
/// so the shader can pick the right sheen/Fresnel/SSS treatment. The caller
/// supplies the per-instance tint via `base_color`.
pub fn talisman_material(
    kind: crate::core::talisman::TalismanKind,
    base_color: [f32; 4],
) -> MaterialParams {
    use crate::core::talisman::TalismanKind as T;
    let (mat_kind, spec_strength, spec_power) = match kind {
        T::Jade => (MaterialKind::Jade, 0.55, 48.0),
        T::Pearl => (MaterialKind::Pearl, 0.60, 64.0),
        T::Gilded => (MaterialKind::GoldNugget, 0.95, 160.0),
        T::Polychrome => (MaterialKind::Polychrome, 0.70, 32.0),
        T::Kiln => (MaterialKind::Polychrome, 0.70, 32.0),
        T::Bamboo | T::Dots | T::Characters | T::Conformity => {
            (MaterialKind::Moonstone, 0.80, 80.0)
        }
        T::Honors | T::Wildflower => (MaterialKind::Pearl, 0.60, 64.0),
    };
    MaterialParams {
        kind: mat_kind,
        base_color,
        specular_strength: spec_strength,
        specular_power: spec_power,
    }
}
