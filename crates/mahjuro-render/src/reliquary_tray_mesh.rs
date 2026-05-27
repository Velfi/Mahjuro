//! Procedural mesh for the Reliquary tray — a wide, shallow lacquered
//! surface with a brass rim that sits beneath the structure-bank melds in
//! the gameplay scene.
//!
//! The tray is a rounded rectangle rather than the disc used by the bronze
//! mirror: melds arrange in a row, so the underlying object needs to be
//! wider than it is tall. Per-instance scale (set by the caller) stretches
//! the unit-space mesh to cover the current bank footprint.
//!
//! # Status
//!
//! Phase 1 of the Reliquary visual overhaul is in progress. This mesh is
//! built but **not yet instantiated** by the gameplay scene — the next
//! session will slip a `ShowcaseObject3d` placement into
//! `scenes::gameplay::*` directly below the existing meld showcase strip.
//!
//! Art direction (see `docs/design_todo_prompts.md` once the Reliquary
//! section lands): WALNUT_INK base, BRASS inner rim, slight Z-lift off the
//! felt so it casts a shadow. The depth-multiplier glow (Phase 2) will
//! pulse the brass rim via an emissive multiplier once the shader gains
//! a per-instance emissive channel.
//!
//! # Coordinates
//!
//! Local space is `-0.5..+0.5` on X (width) and Z (depth), with a tiny
//! Y-range (`BOT_Y..TOP_Y`) for the slab thickness. This matches the
//! mirror/bowl convention so per-instance scale supplies the true size.

use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::theme::color;
use crate::tile_glb::Vertex3dTex;

/// Half-width of the tray body in local space.
const HALF_W: f32 = 0.50;
/// Half-depth of the tray body in local space. Shallow compared to the
/// width so the footprint reads as a long rectangular tray.
const HALF_D: f32 = 0.18;
/// Inset from the outer edge where the brass rim ends and the inner
/// lacquered face begins.
const RIM_INSET: f32 = 0.035;
/// Y of the tray's underside.
const BOT_Y: f32 = -0.02;
/// Y of the top of the brass rim.
const TOP_Y: f32 = 0.02;
/// Y of the recessed inner lacquered face — below the rim so the rim
/// reads as a raised frame.
const FACE_Y: f32 = 0.008;

/// Local AABB half-extents of the tray mesh, for the raycast picker when
/// hover-highlight lands in a future phase.
#[allow(dead_code)]
pub const RELIQUARY_LOCAL_HALF: [f32; 3] = [HALF_W, (TOP_Y - BOT_Y) * 0.5, HALF_D];
#[allow(dead_code)]
pub const RELIQUARY_LOCAL_CENTER_Y: f32 = (TOP_Y + BOT_Y) * 0.5;

/// Build the Reliquary tray — a flat lacquered rectangle with a brass
/// rim. The material is routed through the `LacqueredWood` shader branch
/// for the body; the rim is emitted in the same mesh but would be
/// overridden by a separate draw if we wanted a distinct rim material.
/// For now, a single-material draw keeps things simple and readable.
#[allow(dead_code)]
pub fn build_reliquary_tray_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Outer rectangle corners (at TOP_Y).
    let outer: [(f32, f32); 4] = [
        (-HALF_W, -HALF_D),
        (HALF_W, -HALF_D),
        (HALF_W, HALF_D),
        (-HALF_W, HALF_D),
    ];
    // Inner rectangle corners (also at TOP_Y, at the rim-face boundary).
    let in_w = HALF_W - RIM_INSET;
    let in_d = HALF_D - RIM_INSET;
    let inner: [(f32, f32); 4] = [(-in_w, -in_d), (in_w, -in_d), (in_w, in_d), (-in_w, in_d)];

    let up = [0.0, 1.0, 0.0];
    let down = [0.0, -1.0, 0.0];

    // Outward-facing normal for each of the four outer walls. Order
    // matches `outer` corner indices: wall i bridges outer[i]→outer[i+1].
    let wall_normals: [[f32; 3]; 4] = [
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [-1.0, 0.0, 0.0],
    ];

    // ── Outer side walls: four upright quads ──
    for i in 0..4 {
        let j = (i + 1) % 4;
        let (x0, z0) = outer[i];
        let (x1, z1) = outer[j];
        let n = wall_normals[i];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [x0, TOP_Y, z0],
            normal: n,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, TOP_Y, z1],
            normal: n,
            uv: [1.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, BOT_Y, z1],
            normal: n,
            uv: [1.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x0, BOT_Y, z0],
            normal: n,
            uv: [0.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Top rim: four quads between outer and inner rectangles at TOP_Y ──
    for i in 0..4 {
        let j = (i + 1) % 4;
        let (ox0, oz0) = outer[i];
        let (ox1, oz1) = outer[j];
        let (ix1, iz1) = inner[j];
        let (ix0, iz0) = inner[i];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [ox0, TOP_Y, oz0],
            normal: up,
            uv: [ox0 / (2.0 * HALF_W) + 0.5, oz0 / (2.0 * HALF_D) + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ox1, TOP_Y, oz1],
            normal: up,
            uv: [ox1 / (2.0 * HALF_W) + 0.5, oz1 / (2.0 * HALF_D) + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, TOP_Y, iz1],
            normal: up,
            uv: [ix1 / (2.0 * HALF_W) + 0.5, iz1 / (2.0 * HALF_D) + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix0, TOP_Y, iz0],
            normal: up,
            uv: [ix0 / (2.0 * HALF_W) + 0.5, iz0 / (2.0 * HALF_D) + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Inner bevel: short slanted quads from inner@TOP_Y down to inner@FACE_Y ──
    for i in 0..4 {
        let j = (i + 1) % 4;
        let (ix0, iz0) = inner[i];
        let (ix1, iz1) = inner[j];
        // Inward-and-up normal.
        let inward = wall_normals[i];
        let n = [-inward[0] * 0.6, 0.8, -inward[2] * 0.6];
        let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        let n = [n[0] / nl, n[1] / nl, n[2] / nl];
        let base = vertices.len() as u32;
        vertices.push(Vertex3dTex {
            position: [ix0, TOP_Y, iz0],
            normal: n,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, TOP_Y, iz1],
            normal: n,
            uv: [1.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix1, FACE_Y, iz1],
            normal: n,
            uv: [1.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [ix0, FACE_Y, iz0],
            normal: n,
            uv: [0.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // ── Inner face: one flat quad at FACE_Y filling the inner rectangle ──
    let base = vertices.len() as u32;
    for &(x, z) in inner.iter() {
        vertices.push(Vertex3dTex {
            position: [x, FACE_Y, z],
            normal: up,
            uv: [x / (2.0 * HALF_W) + 0.5, z / (2.0 * HALF_D) + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    // ── Underside: one flat quad at BOT_Y so the tray has a closed back ──
    let base = vertices.len() as u32;
    for &(x, z) in outer.iter().rev() {
        vertices.push(Vertex3dTex {
            position: [x, BOT_Y, z],
            normal: down,
            uv: [x / (2.0 * HALF_W) + 0.5, z / (2.0 * HALF_D) + 0.5],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    MeshCpu {
        vertices,
        indices,
        // Deep obsidian-lacquer body — `WALNUT_INK` reads as the cool
        // counterpoint to the surrounding warm walnut. Phase 2 will add a
        // brass rim overlay and emissive pulse driven by the structure
        // bank's depth multiplier.
        default_material: MaterialParams {
            kind: MaterialKind::LacqueredWood,
            base_color: color::WALNUT_INK,
            specular_strength: 0.55,
            specular_power: 96.0,
        },
    }
}
