//! Procedural meshes for the on-table relic dish + the placeholder relic boxes
//! that sit inside it. Both are simple axis-aligned boxes built once at startup
//! and instanced via [`crate::render::lit_mesh::LitMeshInstance`].
//!
//! The dish is a flat low box centered at local (0,0,0); the relic placeholder
//! is a unit cube spanning -0.5..+0.5 on each axis so a per-instance scale can
//! turn it into rectangular prisms of varying sizes.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

/// Build a unit-cube mesh: 6 quads, axis-aligned, spanning -0.5..+0.5 in x/y/z.
/// Each face has its own normals so lighting reads as flat-shaded.
pub fn build_unit_box_mesh() -> MeshCpu {
    // 6 faces × 4 vertices, with per-face normals.
    // Face order: +X, -X, +Y, -Y, +Z, -Z.
    let faces: &[([f32; 3], [[f32; 3]; 4])] = &[
        // +X (right)
        (
            [1.0, 0.0, 0.0],
            [
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
                [0.5, -0.5, 0.5],
            ],
        ),
        // -X (left)
        (
            [-1.0, 0.0, 0.0],
            [
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, -0.5],
            ],
        ),
        // +Y (top)
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, 0.5, -0.5],
                [-0.5, 0.5, 0.5],
                [0.5, 0.5, 0.5],
                [0.5, 0.5, -0.5],
            ],
        ),
        // -Y (bottom)
        (
            [0.0, -1.0, 0.0],
            [
                [-0.5, -0.5, 0.5],
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, -0.5, 0.5],
            ],
        ),
        // +Z (front, toward camera)
        (
            [0.0, 0.0, 1.0],
            [
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
        ),
        // -Z (back)
        (
            [0.0, 0.0, -1.0],
            [
                [0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
            ],
        ),
    ];

    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity(24);
    let mut indices: Vec<u32> = Vec::with_capacity(36);

    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *corner,
                normal: *normal,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.85, 0.78, 0.55, 1.0],
            specular_strength: 0.35,
            specular_power: 24.0,
        },
    }
}

/// Build the dish mesh: a shallow brass tray with a raised rim around its
/// top edge and a recessed floor in the middle. Local units span -0.5..0.5
/// on each axis (same convention as the unit box) so callers can scale it
/// independently in x/y/z to choose the dish footprint and rim height.
///
/// Geometry: outer footprint is the full -0.5..0.5 cube. The top face is
/// replaced by an annular rim (a flat ring at y=+0.5) plus four inward-facing
/// inner walls dropping down to a recessed floor at y=`RECESS_FLOOR`. Things
/// placed "on" the dish (coins, relic placeholders) still sit on top of the
/// rim because callers anchor them at +half_height — the recess only affects
/// what the player sees inside the lip.
pub fn build_dish_mesh() -> MeshCpu {
    // Wall thickness in xz (fraction of full extent) and recess floor height
    // (in local y, where the rim top sits at +0.5 and the dish base at -0.5).
    const RIM_THICK: f32 = 0.10;
    const RECESS_FLOOR: f32 = 0.20;
    let inner = 0.5 - RIM_THICK;
    let rim_top = 0.5_f32;

    // Each face is a quad with explicit corners listed CCW when viewed from
    // along the normal. Same convention as `build_unit_box_mesh`.
    let faces: &[([f32; 3], [[f32; 3]; 4])] = &[
        // ── Outer shell ────────────────────────────────────────────────
        // Bottom (-Y)
        (
            [0.0, -1.0, 0.0],
            [
                [-0.5, -0.5, 0.5],
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, -0.5, 0.5],
            ],
        ),
        // Outer +X
        (
            [1.0, 0.0, 0.0],
            [
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
                [0.5, -0.5, 0.5],
            ],
        ),
        // Outer -X
        (
            [-1.0, 0.0, 0.0],
            [
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, -0.5],
            ],
        ),
        // Outer +Z
        (
            [0.0, 0.0, 1.0],
            [
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
        ),
        // Outer -Z
        (
            [0.0, 0.0, -1.0],
            [
                [0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
            ],
        ),
        // ── Rim top: annular ring at y=+0.5 (4 strips) ────────────────
        // North strip (+Z side of the rim)
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, rim_top, inner],
                [-0.5, rim_top, 0.5],
                [0.5, rim_top, 0.5],
                [0.5, rim_top, inner],
            ],
        ),
        // South strip (-Z side of the rim)
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, rim_top, -0.5],
                [-0.5, rim_top, -inner],
                [0.5, rim_top, -inner],
                [0.5, rim_top, -0.5],
            ],
        ),
        // East strip (+X side of the rim, between the N/S strips)
        (
            [0.0, 1.0, 0.0],
            [
                [inner, rim_top, -inner],
                [inner, rim_top, inner],
                [0.5, rim_top, inner],
                [0.5, rim_top, -inner],
            ],
        ),
        // West strip (-X side of the rim, between the N/S strips)
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, rim_top, -inner],
                [-0.5, rim_top, inner],
                [-inner, rim_top, inner],
                [-inner, rim_top, -inner],
            ],
        ),
        // ── Inner walls (face inward toward the recess) ───────────────
        // Inner +X wall at x=+inner, normal -X
        (
            [-1.0, 0.0, 0.0],
            [
                [inner, RECESS_FLOOR, inner],
                [inner, rim_top, inner],
                [inner, rim_top, -inner],
                [inner, RECESS_FLOOR, -inner],
            ],
        ),
        // Inner -X wall at x=-inner, normal +X
        (
            [1.0, 0.0, 0.0],
            [
                [-inner, RECESS_FLOOR, -inner],
                [-inner, rim_top, -inner],
                [-inner, rim_top, inner],
                [-inner, RECESS_FLOOR, inner],
            ],
        ),
        // Inner +Z wall at z=+inner, normal -Z
        (
            [0.0, 0.0, -1.0],
            [
                [-inner, RECESS_FLOOR, inner],
                [-inner, rim_top, inner],
                [inner, rim_top, inner],
                [inner, RECESS_FLOOR, inner],
            ],
        ),
        // Inner -Z wall at z=-inner, normal +Z
        (
            [0.0, 0.0, 1.0],
            [
                [inner, RECESS_FLOOR, -inner],
                [inner, rim_top, -inner],
                [-inner, rim_top, -inner],
                [-inner, RECESS_FLOOR, -inner],
            ],
        ),
        // ── Recess floor at y=RECESS_FLOOR, normal +Y ─────────────────
        (
            [0.0, 1.0, 0.0],
            [
                [-inner, RECESS_FLOOR, -inner],
                [-inner, RECESS_FLOOR, inner],
                [inner, RECESS_FLOOR, inner],
                [inner, RECESS_FLOOR, -inner],
            ],
        ),
    ];

    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity(faces.len() * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(faces.len() * 6);
    let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *corner,
                normal: *normal,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    MeshCpu {
        vertices,
        indices,
        // Darker brass tint — matches the Midnight Gold palette and reads
        // as a recessed metal tray under candlelight.
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.32, 0.22, 0.10, 1.0],
            specular_strength: 0.55,
            specular_power: 48.0,
        },
    }
}
