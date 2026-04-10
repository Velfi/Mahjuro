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

/// Build a book mesh: a rectangular box with a rounded spine on the −X edge
/// and a narrow page-block inset visible on the other three open edges
/// (+X, +Z, −Z). The mesh spans −0.5..+0.5 on each axis so callers can
/// scale it into the desired proportions (width × height × depth).
///
/// The spine is approximated with 6 arc segments to give it a convincing
/// curved look under the lit-mesh shader. The page inset is a thinner
/// inner box offset from the cover by `PAGE_INSET` on three sides,
/// recessed by `COVER_THICK` in Y, giving the silhouette of paper pages
/// peeking out between the covers.
pub fn build_book_mesh() -> MeshCpu {
    // How far the spine bulges past the cover on the −X side.
    const SPINE_BULGE: f32 = 0.08;
    // Number of arc segments in the spine curve.
    const SPINE_SEGS: usize = 6;
    // Cover thickness (how far pages are recessed from the cover surface).
    const COVER_THICK: f32 = 0.06;
    // How far pages are inset from the cover edge on +X / +Z / −Z.
    const PAGE_INSET: f32 = 0.02;

    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Helper: push a quad (4 verts, 6 indices) with a flat normal.
    let mut push_quad = |normal: [f32; 3], corners: [[f32; 3]; 4]| {
        let base = vertices.len() as u32;
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        for (c, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *c,
                normal,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    // ── Cover box (outer shell) ───────────────────────────────────────
    // +X face (open edge of pages)
    push_quad(
        [1.0, 0.0, 0.0],
        [
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [0.5, 0.5, 0.5],
            [0.5, -0.5, 0.5],
        ],
    );
    // +Y face (front cover)
    push_quad(
        [0.0, 1.0, 0.0],
        [
            [-0.5, 0.5, -0.5],
            [-0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [0.5, 0.5, -0.5],
        ],
    );
    // −Y face (back cover)
    push_quad(
        [0.0, -1.0, 0.0],
        [
            [-0.5, -0.5, 0.5],
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, -0.5, 0.5],
        ],
    );
    // +Z face (top edge)
    push_quad(
        [0.0, 0.0, 1.0],
        [
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ],
    );
    // −Z face (bottom edge)
    push_quad(
        [0.0, 0.0, -1.0],
        [
            [0.5, -0.5, -0.5],
            [-0.5, -0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [0.5, 0.5, -0.5],
        ],
    );

    // ── Spine (rounded −X edge) ───────────────────────────────────────
    // Arc from (−0.5, −0.5) to (−0.5, +0.5) in the XY plane, bulging
    // out to x = −0.5 − SPINE_BULGE at the midpoint. Each segment is a
    // quad strip running along z (−0.5..+0.5).
    {
        let cx = -0.5_f32; // arc center x (flat cover edge)
        let cy = 0.0_f32; // arc center y
        let ry = 0.5_f32; // half-height of the cover
        let rx = SPINE_BULGE; // how far the spine pokes out
        for seg in 0..SPINE_SEGS {
            let t0 = seg as f32 / SPINE_SEGS as f32;
            let t1 = (seg + 1) as f32 / SPINE_SEGS as f32;
            // Angles from π/2 (top, +Y) to −π/2 (bottom, −Y), going
            // through π (leftward bulge).
            let a0 = std::f32::consts::FRAC_PI_2 - t0 * std::f32::consts::PI;
            let a1 = std::f32::consts::FRAC_PI_2 - t1 * std::f32::consts::PI;
            let x0 = cx - rx * a0.cos().abs();
            let y0 = cy + ry * a0.sin();
            let x1 = cx - rx * a1.cos().abs();
            let y1 = cy + ry * a1.sin();
            // Normal points outward from the arc center.
            let nx0 = -a0.cos();
            let ny0 = a0.sin();
            let nx1 = -a1.cos();
            let ny1 = a1.sin();
            let nmx = (nx0 + nx1) * 0.5;
            let nmy = (ny0 + ny1) * 0.5;
            let len = (nmx * nmx + nmy * nmy).sqrt().max(0.001);
            let normal = [nmx / len, nmy / len, 0.0];
            push_quad(
                normal,
                [[x0, y0, -0.5], [x1, y1, -0.5], [x1, y1, 0.5], [x0, y0, 0.5]],
            );
        }
    }

    // ── Page block (inner lighter box on the open edge) ───────────────
    // Visible on +X, +Z, −Z as a recessed cream-colored band. The page
    // block sits between the two covers (y = ±(0.5 − COVER_THICK)) and
    // is inset from the cover edges.
    let py = 0.5 - COVER_THICK; // page half-thickness in Y
    let pz = 0.5 - PAGE_INSET; // page half-depth in Z
    let px_inner = -0.5 + PAGE_INSET; // page start on −X side
    let px_outer = 0.5 - PAGE_INSET; // page end on +X side
    // +X face of pages (the fore-edge)
    push_quad(
        [1.0, 0.0, 0.0],
        [
            [px_outer, -py, -pz],
            [px_outer, py, -pz],
            [px_outer, py, pz],
            [px_outer, -py, pz],
        ],
    );
    // +Z face of pages (top edge pages)
    push_quad(
        [0.0, 0.0, 1.0],
        [
            [px_inner, -py, pz],
            [px_outer, -py, pz],
            [px_outer, py, pz],
            [px_inner, py, pz],
        ],
    );
    // −Z face of pages (bottom edge pages)
    push_quad(
        [0.0, 0.0, -1.0],
        [
            [px_outer, -py, -pz],
            [px_inner, -py, -pz],
            [px_inner, py, -pz],
            [px_outer, py, -pz],
        ],
    );
    // +Y face of pages (front page surface, recessed below front cover)
    push_quad(
        [0.0, 1.0, 0.0],
        [
            [px_inner, py, -pz],
            [px_inner, py, pz],
            [px_outer, py, pz],
            [px_outer, py, -pz],
        ],
    );
    // −Y face of pages (back page surface, recessed below back cover)
    push_quad(
        [0.0, -1.0, 0.0],
        [
            [px_inner, -py, pz],
            [px_inner, -py, -pz],
            [px_outer, -py, -pz],
            [px_outer, -py, pz],
        ],
    );

    MeshCpu {
        vertices,
        indices,
        // Deep oxblood / maroon cover — reads as aged leather under warm
        // candlelight. The page block will be tinted at render time via
        // a per-instance material override (cream/ivory).
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.30, 0.12, 0.08, 1.0],
            specular_strength: 0.20,
            specular_power: 16.0,
        },
    }
}
