//! Procedural mesh for the discard bowl — a low lacquered-wood bowl with
//! a flat rim. Used by the gameplay scene's Discard action.
//!
//! The bowl is built as a 16-sided revolved cylinder shell: an outer wall
//! (visible from outside), an inner wall (visible from above when looking
//! into the bowl), a flat ring rim joining them at the top, and a flat
//! bottom disc. The interior is hollow so a tile dropped in reads as
//! sitting inside the cup.
//!
//! Local space spans `-0.5..+0.5` on X/Z (radius) and `-0.5..+0.5` on Y
//! (height) so per-instance scale supplies the actual size.

use std::f32::consts::TAU;

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

const SIDES: usize = 16;
/// Wall thickness as a fraction of the outer radius.
const WALL: f32 = 0.18;
/// Inner radius (= 0.5 * (1 - 2*WALL) is too thin; use a single-sided ratio).
const INNER_R: f32 = 0.5 - WALL;
/// Outer radius.
const OUTER_R: f32 = 0.5;
/// Bowl bottom Y (slightly above local floor so the bottom disc casts properly).
const BOTTOM_Y: f32 = -0.45;
/// Bowl top Y (the rim).
const TOP_Y: f32 = 0.45;

/// Build a low lacquered-wood bowl mesh.
pub fn build_bowl_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Pre-compute ring positions for inner + outer radii.
    let outer: Vec<(f32, f32)> = (0..SIDES)
        .map(|i| {
            let theta = (i as f32) * TAU / SIDES as f32;
            (OUTER_R * theta.cos(), OUTER_R * theta.sin())
        })
        .collect();
    let inner: Vec<(f32, f32)> = (0..SIDES)
        .map(|i| {
            let theta = (i as f32) * TAU / SIDES as f32;
            (INNER_R * theta.cos(), INNER_R * theta.sin())
        })
        .collect();

    // ── Outer wall: 16 quads with outward-facing normals ──
    for i in 0..SIDES {
        let (x0, z0) = outer[i];
        let (x1, z1) = outer[(i + 1) % SIDES];
        let mx = (x0 + x1) * 0.5;
        let mz = (z0 + z1) * 0.5;
        let len = (mx * mx + mz * mz).sqrt().max(1e-6);
        let n = [mx / len, 0.0, mz / len];
        let base = vertices.len() as u32;
        // top0, top1, bot1, bot0
        vertices.push(Vertex3dTex { position: [x0, TOP_Y, z0], normal: n, uv: [0.0, 0.0] });
        vertices.push(Vertex3dTex { position: [x1, TOP_Y, z1], normal: n, uv: [1.0, 0.0] });
        vertices.push(Vertex3dTex { position: [x1, BOTTOM_Y, z1], normal: n, uv: [1.0, 1.0] });
        vertices.push(Vertex3dTex { position: [x0, BOTTOM_Y, z0], normal: n, uv: [0.0, 1.0] });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Inner wall: 16 quads with inward-facing normals (opposite winding) ──
    for i in 0..SIDES {
        let (x0, z0) = inner[i];
        let (x1, z1) = inner[(i + 1) % SIDES];
        let mx = (x0 + x1) * 0.5;
        let mz = (z0 + z1) * 0.5;
        let len = (mx * mx + mz * mz).sqrt().max(1e-6);
        let n = [-mx / len, 0.0, -mz / len];
        let base = vertices.len() as u32;
        // Reversed winding so inside faces inward.
        vertices.push(Vertex3dTex { position: [x1, TOP_Y, z1], normal: n, uv: [0.0, 0.0] });
        vertices.push(Vertex3dTex { position: [x0, TOP_Y, z0], normal: n, uv: [1.0, 0.0] });
        vertices.push(Vertex3dTex { position: [x0, BOTTOM_Y, z0], normal: n, uv: [1.0, 1.0] });
        vertices.push(Vertex3dTex { position: [x1, BOTTOM_Y, z1], normal: n, uv: [0.0, 1.0] });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Top rim ring: a flat annulus joining inner and outer at TOP_Y ──
    let rim_normal = [0.0, 1.0, 0.0];
    for i in 0..SIDES {
        let (ox0, oz0) = outer[i];
        let (ox1, oz1) = outer[(i + 1) % SIDES];
        let (ix0, iz0) = inner[i];
        let (ix1, iz1) = inner[(i + 1) % SIDES];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex { position: [ox0, TOP_Y, oz0], normal: rim_normal, uv: [0.0, 0.0] });
        vertices.push(Vertex3dTex { position: [ox1, TOP_Y, oz1], normal: rim_normal, uv: [1.0, 0.0] });
        vertices.push(Vertex3dTex { position: [ix1, TOP_Y, iz1], normal: rim_normal, uv: [1.0, 1.0] });
        vertices.push(Vertex3dTex { position: [ix0, TOP_Y, iz0], normal: rim_normal, uv: [0.0, 1.0] });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Bottom disc: a flat fan filling the inside floor (visible from above) ──
    let bottom_normal = [0.0, 1.0, 0.0];
    let center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, BOTTOM_Y, 0.0],
        normal: bottom_normal,
        uv: [0.5, 0.5],
    });
    let ring_start = vertices.len() as u32;
    for &(x, z) in inner.iter() {
        vertices.push(Vertex3dTex {
            position: [x, BOTTOM_Y, z],
            normal: bottom_normal,
            uv: [x + 0.5, z + 0.5],
        });
    }
    for i in 0..SIDES {
        let i0 = ring_start + i as u32;
        let i1 = ring_start + ((i + 1) % SIDES) as u32;
        // Wind so the +Y normal matches.
        indices.extend_from_slice(&[center_idx, i1, i0]);
    }

    // ── Outer base disc: a downward-facing fan so the bowl has a closed
    //    underside (catches shadow correctly).
    let under_normal = [0.0, -1.0, 0.0];
    let under_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, BOTTOM_Y - 0.02, 0.0],
        normal: under_normal,
        uv: [0.5, 0.5],
    });
    let under_ring_start = vertices.len() as u32;
    for &(x, z) in outer.iter() {
        vertices.push(Vertex3dTex {
            position: [x, BOTTOM_Y - 0.02, z],
            normal: under_normal,
            uv: [x + 0.5, z + 0.5],
        });
    }
    for i in 0..SIDES {
        let i0 = under_ring_start + i as u32;
        let i1 = under_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[under_center, i0, i1]);
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::LacqueredWood,
            base_color: [1.0, 1.0, 1.0, 1.0],
            specular_strength: 0.55,
            specular_power: 96.0,
        },
    }
}
