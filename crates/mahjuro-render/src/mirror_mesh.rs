//! Procedural mesh for the bronze mirror — a polished circular bronze
//! disc that sits flat on the table as the visual counterpart to the
//! discard bowl. Clicking it plays the selected hand.
//!
//! Built as a 24-sided revolved disc: a thick outer rim wall, a flat
//! bronze rim ring on top, a slightly recessed polished mirror face
//! inside the rim, and a closed underside. The face is recessed below
//! the rim so the silhouette reads as "framed mirror" rather than a
//! plain coin from across the table.
//!
//! Local space spans `-0.5..+0.5` on X/Z (radius) and `-0.5..+0.5` on Y
//! (height) so per-instance scale supplies the actual size, matching the
//! bowl mesh convention.

use std::f32::consts::TAU;

use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::tile_glb::Vertex3dTex;

const SIDES: usize = 24;
/// Outer radius of the bronze frame.
const OUTER_R: f32 = 0.50;
/// Inner radius — the edge of the polished mirror face (where the rim
/// frame ends).
const INNER_R: f32 = 0.42;
/// Underside of the disc.
const BOT_Y: f32 = -0.03;
/// Top of the bronze rim frame.
const TOP_Y: f32 = 0.03;
/// Polished mirror face — slightly recessed below the rim so the rim
/// reads as a raised frame.
const FACE_Y: f32 = 0.012;

/// Local-space AABB half-extents tight to the mirror mesh, used by the
/// gameplay raycast picker. Center offset on Y is `(TOP_Y + BOT_Y) * 0.5`.
pub const MIRROR_LOCAL_HALF: [f32; 3] = [OUTER_R, (TOP_Y - BOT_Y) * 0.5, OUTER_R];
pub const MIRROR_LOCAL_CENTER_Y: f32 = (TOP_Y + BOT_Y) * 0.5;

/// Build a flat polished bronze mirror disc.
pub fn build_mirror_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let ring = |radius: f32| -> Vec<(f32, f32)> {
        (0..SIDES)
            .map(|i| {
                let theta = (i as f32) * TAU / SIDES as f32;
                (radius * theta.cos(), radius * theta.sin())
            })
            .collect()
    };
    let outer = ring(OUTER_R);
    let inner = ring(INNER_R);

    // Per-vertex outward radial normals shared between adjacent quads so
    // the rim wall reads as smooth under the lit shader (same trick the
    // bowl mesh uses).
    let outer_normals: Vec<[f32; 3]> = (0..SIDES)
        .map(|i| {
            let (x, z) = outer[i];
            let len = (x * x + z * z).sqrt().max(1e-6);
            [x / len, 0.0, z / len]
        })
        .collect();

    // ── Outer rim wall: 24 vertical quads ──
    for i in 0..SIDES {
        let i1 = (i + 1) % SIDES;
        let (x0, z0) = outer[i];
        let (x1, z1) = outer[i1];
        let n0 = outer_normals[i];
        let n1 = outer_normals[i1];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [x0, TOP_Y, z0],
            normal: n0,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, TOP_Y, z1],
            normal: n1,
            uv: [1.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, BOT_Y, z1],
            normal: n1,
            uv: [1.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x0, BOT_Y, z0],
            normal: n0,
            uv: [0.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Top bronze rim ring (annulus at TOP_Y) ──
    //
    // UVs are disc-projected (`x + 0.5, z + 0.5`) — the same scheme the
    // recessed face plate uses below — so a single circular heightmap
    // texture wraps continuously across rim + face as if projected from
    // above. Each rim quad therefore samples the rim region of the
    // heightmap (raised white in the four-spirit composition) instead of
    // an arbitrary vertical strip; this is what keeps the metal-shader
    // heightmap perturbation in lit_mesh.wgsl from twisting the bronze
    // frame when it samples the texture on flat-up faces.
    let up = [0.0, 1.0, 0.0];
    for i in 0..SIDES {
        let i1 = (i + 1) % SIDES;
        let (ox0, oz0) = outer[i];
        let (ox1, oz1) = outer[i1];
        let (ix0, iz0) = inner[i];
        let (ix1, iz1) = inner[i1];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [ox0, TOP_Y, oz0],
            normal: up,
            uv: [ox0 + 0.5, oz0 + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ox1, TOP_Y, oz1],
            normal: up,
            uv: [ox1 + 0.5, oz1 + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, TOP_Y, iz1],
            normal: up,
            uv: [ix1 + 0.5, iz1 + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix0, TOP_Y, iz0],
            normal: up,
            uv: [ix0 + 0.5, iz0 + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Inner bevel: short slanted ring connecting the top rim's inner
    //    edge down to the recessed mirror face plane. This is what makes
    //    the face read as inset rather than flush with the frame.
    for i in 0..SIDES {
        let i1 = (i + 1) % SIDES;
        let (ix0, iz0) = inner[i];
        let (ix1, iz1) = inner[i1];
        // Tilt the bevel normal slightly inward + upward.
        let bevel_normal = |x: f32, z: f32| -> [f32; 3] {
            let len = (x * x + z * z).sqrt().max(1e-6);
            let nx = -x / len * 0.5;
            let nz = -z / len * 0.5;
            let ny = 0.5;
            let nl = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            [nx / nl, ny / nl, nz / nl]
        };
        let n0 = bevel_normal(ix0, iz0);
        let n1 = bevel_normal(ix1, iz1);
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [ix0, TOP_Y, iz0],
            normal: n0,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, TOP_Y, iz1],
            normal: n1,
            uv: [1.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, FACE_Y, iz1],
            normal: n1,
            uv: [1.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix0, FACE_Y, iz0],
            normal: n0,
            uv: [0.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Polished mirror face: a flat fan at FACE_Y filling the inner ring.
    let face_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, FACE_Y, 0.0],
        normal: up,
        uv: [0.5, 0.5],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let face_ring_start = vertices.len() as u32;
    for &(x, z) in inner.iter() {
        vertices.push(Vertex3dTex {
            position: [x, FACE_Y, z],
            normal: up,
            uv: [x + 0.5, z + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..SIDES {
        let i0 = face_ring_start + i as u32;
        let i1 = face_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[face_center, i0, i1]);
    }

    // ── Underside: downward-facing fan so the disc has a closed back.
    let down = [0.0, -1.0, 0.0];
    let under_center = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, BOT_Y, 0.0],
        normal: down,
        uv: [0.5, 0.5],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let under_ring_start = vertices.len() as u32;
    for &(x, z) in outer.iter() {
        vertices.push(Vertex3dTex {
            position: [x, BOT_Y, z],
            normal: down,
            uv: [x + 0.5, z + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..SIDES {
        let i0 = under_ring_start + i as u32;
        let i1 = under_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[under_center, i1, i0]);
    }

    MeshCpu {
        vertices,
        indices,
        // Polished bronze: warm orange-gold base with a high specular
        // strength so it catches the candle highlights and reads as
        // metal. Routed through the Metal shader path for the Schlick
        // Fresnel response.
        default_material: MaterialParams {
            kind: MaterialKind::Metal,
            base_color: [0.78, 0.48, 0.18, 1.0],
            specular_strength: 0.90,
            specular_power: 64.0,
        },
    }
}
