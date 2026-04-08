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

const SIDES: usize = 24;
/// Outer radius at the foot (where the bowl meets the table).
const OUTER_R_BOT: f32 = 0.36;
/// Outer radius at the rim (flared outward — the silhouette that makes a
/// bowl read as a *bowl* rather than a cylinder when viewed from above).
const OUTER_R_TOP: f32 = 0.50;
/// Inner radius at the base of the well.
const INNER_R_BOT: f32 = 0.28;
/// Inner radius at the rim. The inside flares with the outside so the wall
/// thickness stays roughly even.
const INNER_R_TOP: f32 = 0.42;
/// Bowl floor (the well).
const WELL_Y: f32 = -0.12;
/// Bowl outer foot — slightly below the well so the underside disc is
/// visually recessed and the bowl reads as resting on a small base.
const FOOT_Y: f32 = -0.18;
/// Rim height. Total local height is `TOP_Y - FOOT_Y = 0.30`, so the bowl
/// is roughly three times as wide as it is tall — clearly a bowl, not a cup.
const TOP_Y: f32 = 0.12;

/// Local-space AABB half-extents tight to the bowl mesh, used by the
/// gameplay raycast picker. Center offset on Y is `(TOP_Y + FOOT_Y) * 0.5`.
pub const BOWL_LOCAL_HALF: [f32; 3] = [OUTER_R_TOP, (TOP_Y - FOOT_Y) * 0.5, OUTER_R_TOP];
pub const BOWL_LOCAL_CENTER_Y: f32 = (TOP_Y + FOOT_Y) * 0.5;

/// Build a low, flared lacquered bowl. The local-space proportions encode
/// the bowl shape (wide rim, shallow well) so the gameplay scene can scale
/// it uniformly without losing the silhouette.
pub fn build_bowl_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Pre-compute the four rings (inner/outer × top/bottom).
    let ring = |radius: f32| -> Vec<(f32, f32)> {
        (0..SIDES)
            .map(|i| {
                let theta = (i as f32) * TAU / SIDES as f32;
                (radius * theta.cos(), radius * theta.sin())
            })
            .collect()
    };
    let outer_top = ring(OUTER_R_TOP);
    let outer_bot = ring(OUTER_R_BOT);
    let inner_top = ring(INNER_R_TOP);
    let inner_bot = ring(INNER_R_BOT);

    // Per-vertex radial normal for a slanted wall, evaluated at one ring
    // vertex (not a quad midpoint). Sharing the same normal across the two
    // quads that meet at this vertex is what makes the revolved wall read
    // as smooth instead of faceted under the lit shader's interpolation.
    let slanted_outer_normal = |x: f32, z: f32, dr: f32| -> [f32; 3] {
        let len = (x * x + z * z).sqrt().max(1e-6);
        let rx = x / len;
        let rz = z / len;
        // Slope: how much the radius grows from foot to rim. The flared
        // wall's outward normal tilts upward by this slope.
        let dy = TOP_Y - FOOT_Y;
        let slope = dr / dy.max(1e-6);
        let ny = slope;
        let nl = (rx * rx + ny * ny + rz * rz).sqrt().max(1e-6);
        [rx / nl, ny / nl, rz / nl]
    };

    // Precompute per-ring-index normals so adjacent quads share them.
    let outer_dr = OUTER_R_TOP - OUTER_R_BOT;
    let inner_dr = INNER_R_TOP - INNER_R_BOT;
    let outer_normals: Vec<[f32; 3]> = (0..SIDES)
        .map(|i| {
            // Use the rim vertex direction (top ring) — the radial direction
            // is identical at top and bottom for a given side, so either
            // works; the rim is just a clean canonical pick.
            let (x, z) = outer_top[i];
            slanted_outer_normal(x, z, outer_dr)
        })
        .collect();
    let inner_normals: Vec<[f32; 3]> = (0..SIDES)
        .map(|i| {
            let (x, z) = inner_top[i];
            // Inner normals mirror the outer radial direction (point toward
            // the axis) but keep the upward tilt so light from above bounces
            // into the well surface.
            let n = slanted_outer_normal(x, z, inner_dr);
            [-n[0], n[1], -n[2]]
        })
        .collect();

    // ── Outer wall: 24 quads, slanted outward (foot → rim) ──
    for i in 0..SIDES {
        let i1 = (i + 1) % SIDES;
        let (xt0, zt0) = outer_top[i];
        let (xt1, zt1) = outer_top[i1];
        let (xb0, zb0) = outer_bot[i];
        let (xb1, zb1) = outer_bot[i1];
        let n0 = outer_normals[i];
        let n1 = outer_normals[i1];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [xt0, TOP_Y, zt0],
            normal: n0,
            uv: [0.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [xt1, TOP_Y, zt1],
            normal: n1,
            uv: [1.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [xb1, FOOT_Y, zb1],
            normal: n1,
            uv: [1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [xb0, FOOT_Y, zb0],
            normal: n0,
            uv: [0.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Inner wall: same flare, opposite winding (faces inward) ──
    for i in 0..SIDES {
        let i1 = (i + 1) % SIDES;
        let (xt0, zt0) = inner_top[i];
        let (xt1, zt1) = inner_top[i1];
        let (xb0, zb0) = inner_bot[i];
        let (xb1, zb1) = inner_bot[i1];
        let n0 = inner_normals[i];
        let n1 = inner_normals[i1];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [xt1, TOP_Y, zt1],
            normal: n1,
            uv: [0.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [xt0, TOP_Y, zt0],
            normal: n0,
            uv: [1.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [xb0, WELL_Y, zb0],
            normal: n0,
            uv: [1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [xb1, WELL_Y, zb1],
            normal: n1,
            uv: [0.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Top rim ring: flat annulus at TOP_Y joining outer_top → inner_top.
    //    This is the visible "lip" of the bowl — keeping it generously wide
    //    is what reads as a rim from across the table.
    let rim_normal = [0.0, 1.0, 0.0];
    for i in 0..SIDES {
        let (ox0, oz0) = outer_top[i];
        let (ox1, oz1) = outer_top[(i + 1) % SIDES];
        let (ix0, iz0) = inner_top[i];
        let (ix1, iz1) = inner_top[(i + 1) % SIDES];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [ox0, TOP_Y, oz0],
            normal: rim_normal,
            uv: [0.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [ox1, TOP_Y, oz1],
            normal: rim_normal,
            uv: [1.0, 0.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, TOP_Y, iz1],
            normal: rim_normal,
            uv: [1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix0, TOP_Y, iz0],
            normal: rim_normal,
            uv: [0.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Well floor: a flat fan at WELL_Y filling the bowl's inside bottom.
    let well_normal = [0.0, 1.0, 0.0];
    let well_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, WELL_Y, 0.0],
        normal: well_normal,
        uv: [0.5, 0.5],
    });
    let well_ring_start = vertices.len() as u32;
    for &(x, z) in inner_bot.iter() {
        vertices.push(Vertex3dTex {
            position: [x, WELL_Y, z],
            normal: well_normal,
            uv: [x + 0.5, z + 0.5],
        });
    }
    for i in 0..SIDES {
        let i0 = well_ring_start + i as u32;
        let i1 = well_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[well_center, i1, i0]);
    }

    // ── Outer foot disc: downward-facing fan so the bowl has a closed
    //    underside that catches shadows cleanly.
    let under_normal = [0.0, -1.0, 0.0];
    let under_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, FOOT_Y - 0.01, 0.0],
        normal: under_normal,
        uv: [0.5, 0.5],
    });
    let under_ring_start = vertices.len() as u32;
    for &(x, z) in outer_bot.iter() {
        vertices.push(Vertex3dTex {
            position: [x, FOOT_Y - 0.01, z],
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
        // Distinct material from the walnut wood tablets next to it: a
        // deep oxblood-red lacquer rendered through the Plain shader (the
        // procedural wood path ignores base_color, which would put the
        // bowl in the same family of warm browns as its neighbours and
        // make the shape blur into the action row).
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.42, 0.06, 0.06, 1.0],
            specular_strength: 0.65,
            specular_power: 96.0,
        },
    }
}
