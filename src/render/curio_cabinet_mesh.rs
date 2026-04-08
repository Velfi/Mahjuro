//! Procedural mesh for the shop's curio cabinet (back wall shadow box).
//!
//! The cabinet is built in normalized local space spanning -0.5..+0.5 on
//! each axis so a per-instance scale matrix can size it via the
//! `CurioCabinetPlacement.extents`. Layout:
//!
//! ```text
//!   +Y up                                +X right
//!   ┌────────────────────────────────┐
//!   │ ┌────┬────┬────┐               │  ← top frame
//!   │ │    │    │    │  ribbons hang │
//!   │ ├────┼────┼────┤  on the right │
//!   │ │    │    │    │  half against │
//!   │ └────┴────┴────┘  the back     │
//!   │   relic niches                 │
//!   └────────────────────────────────┘  ← bottom frame
//! ```
//!
//! The left half is subdivided into a `NICHE_COLS x NICHE_ROWS` grid of
//! inset compartments (back panel pushed in by `NICHE_DEPTH` plus dividers
//! between cells). The right half is left as a flat back panel; the shop
//! scene pins ribbons against it.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

/// Number of relic niche columns on the left half.
pub const NICHE_COLS: usize = 3;
/// Number of relic niche rows on the left half.
pub const NICHE_ROWS: usize = 1;
/// Depth (in local units, normalized to 1) of each niche back panel relative
/// to the cabinet front plane.
const NICHE_DEPTH: f32 = 0.55;
/// Frame thickness around the cabinet outer edge.
const FRAME: f32 = 0.04;
/// Thickness of dividers between niches.
const DIVIDER: f32 = 0.025;
/// Half-thickness of the cabinet's front plane (z extent).
const HALF_Z: f32 = 0.5;

/// Build the curio cabinet mesh in local space (-0.5..+0.5 on each axis).
pub fn build_curio_cabinet_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // The cabinet is one big box with several smaller boxes carved into the
    // left half. Building literal CSG would be overkill, so we just emit
    // a set of axis-aligned boxes whose visible faces compose the cabinet:
    //
    // 1. A back panel (the entire cabinet's back face)
    // 2. Outer frame (top, bottom, left, right strips on the front face)
    // 3. A middle vertical divider splitting left/right halves
    // 4. The dividers between niches on the left half
    // 5. The flat back panel of the right half (sits at the cabinet front
    //    plane, since ribbons hang against it)
    //
    // This avoids needing real geometry CSG while still reading as
    // compartmentalized shelving. Each "box" is added via `push_box`.

    // Local extents.
    let (xmin, xmax) = (-0.5, 0.5);
    let (ymin, ymax) = (-0.5, 0.5);
    let (zmin, zmax) = (-HALF_Z, HALF_Z);

    // ── 1. Big back panel: thin slab at the back of the cabinet.
    push_box(
        &mut vertices,
        &mut indices,
        xmin,
        xmax,
        ymin,
        ymax,
        zmin,
        zmin + 0.04,
    );

    // ── 2. Outer frame strips on the front plane.
    let zfront_lo = zmax - 0.06;
    let zfront_hi = zmax;
    // Top strip
    push_box(
        &mut vertices,
        &mut indices,
        xmin,
        xmax,
        ymax - FRAME,
        ymax,
        zfront_lo,
        zfront_hi,
    );
    // Bottom strip
    push_box(
        &mut vertices,
        &mut indices,
        xmin,
        xmax,
        ymin,
        ymin + FRAME,
        zfront_lo,
        zfront_hi,
    );
    // Left strip
    push_box(
        &mut vertices,
        &mut indices,
        xmin,
        xmin + FRAME,
        ymin + FRAME,
        ymax - FRAME,
        zfront_lo,
        zfront_hi,
    );
    // Right strip
    push_box(
        &mut vertices,
        &mut indices,
        xmax - FRAME,
        xmax,
        ymin + FRAME,
        ymax - FRAME,
        zfront_lo,
        zfront_hi,
    );

    // ── 3. Middle divider splitting left/right halves.
    push_box(
        &mut vertices,
        &mut indices,
        -DIVIDER * 0.5,
        DIVIDER * 0.5,
        ymin + FRAME,
        ymax - FRAME,
        zfront_lo,
        zfront_hi,
    );

    // ── 4. Niche dividers on the left half (vertical between columns,
    //         horizontal between rows). Each divider sits flush with the
    //         cabinet front plane.
    let left_x0 = xmin + FRAME;
    let left_x1 = -DIVIDER * 0.5;
    let inner_y0 = ymin + FRAME;
    let inner_y1 = ymax - FRAME;
    let cell_w = (left_x1 - left_x0) / NICHE_COLS as f32;
    let cell_h = (inner_y1 - inner_y0) / NICHE_ROWS as f32;
    // Vertical dividers between columns.
    for c in 1..NICHE_COLS {
        let x = left_x0 + c as f32 * cell_w;
        push_box(
            &mut vertices,
            &mut indices,
            x - DIVIDER * 0.5,
            x + DIVIDER * 0.5,
            inner_y0,
            inner_y1,
            zfront_lo,
            zfront_hi,
        );
    }
    // Horizontal dividers between rows.
    for r in 1..NICHE_ROWS {
        let y = inner_y0 + r as f32 * cell_h;
        push_box(
            &mut vertices,
            &mut indices,
            left_x0,
            left_x1,
            y - DIVIDER * 0.5,
            y + DIVIDER * 0.5,
            zfront_lo,
            zfront_hi,
        );
    }
    // Niche back panels: a thin layer behind each cell at NICHE_DEPTH from
    // the front plane, so each compartment reads as recessed.
    let niche_back_z = zmax - NICHE_DEPTH;
    for r in 0..NICHE_ROWS {
        for c in 0..NICHE_COLS {
            let x0 = left_x0 + c as f32 * cell_w + DIVIDER * 0.5;
            let x1 = left_x0 + (c + 1) as f32 * cell_w - DIVIDER * 0.5;
            let y0 = inner_y0 + r as f32 * cell_h + DIVIDER * 0.5;
            let y1 = inner_y0 + (r + 1) as f32 * cell_h - DIVIDER * 0.5;
            push_box(
                &mut vertices,
                &mut indices,
                x0,
                x1,
                y0,
                y1,
                niche_back_z,
                niche_back_z + 0.02,
            );
        }
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::LacqueredWood,
            base_color: [1.0, 1.0, 1.0, 1.0], // procedural wood
            specular_strength: 0.55,
            specular_power: 96.0,
        },
    }
}

/// Centers (in local space, -0.5..+0.5) of each relic niche on the left half.
/// Returned in row-major order: top→bottom, left→right within each row.
/// `n` is clamped to `NICHE_COLS * NICHE_ROWS`. The Z component is the niche
/// floor (slightly in front of the niche back panel) so a relic placed there
/// reads as "sitting in" the compartment.
pub fn niche_centers_local(n: usize) -> Vec<[f32; 3]> {
    let xmin = -0.5;
    let ymin = -0.5;
    let ymax = 0.5;
    let left_x0 = xmin + FRAME;
    let left_x1 = -DIVIDER * 0.5;
    let inner_y0 = ymin + FRAME;
    let inner_y1 = ymax - FRAME;
    let cell_w = (left_x1 - left_x0) / NICHE_COLS as f32;
    let cell_h = (inner_y1 - inner_y0) / NICHE_ROWS as f32;
    let niche_back_z = 0.5 - NICHE_DEPTH;
    let mut out = Vec::with_capacity(n.min(NICHE_COLS * NICHE_ROWS));
    let mut count = 0;
    'outer: for r in (0..NICHE_ROWS).rev() {
        for c in 0..NICHE_COLS {
            if count >= n {
                break 'outer;
            }
            let cx = left_x0 + (c as f32 + 0.5) * cell_w;
            let cy = inner_y0 + (r as f32 + 0.5) * cell_h;
            let cz = niche_back_z + 0.05;
            out.push([cx, cy, cz]);
            count += 1;
        }
    }
    out
}

/// Append a colored axis-aligned box to (vertices, indices). 6 quads, 24
/// verts (each face has its own normal so the lit shader reads flat).
fn push_box(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    z0: f32,
    z1: f32,
) {
    let faces: &[([f32; 3], [[f32; 3]; 4])] = &[
        // +X
        (
            [1.0, 0.0, 0.0],
            [
                [x1, y0, z0],
                [x1, y1, z0],
                [x1, y1, z1],
                [x1, y0, z1],
            ],
        ),
        // -X
        (
            [-1.0, 0.0, 0.0],
            [
                [x0, y0, z1],
                [x0, y1, z1],
                [x0, y1, z0],
                [x0, y0, z0],
            ],
        ),
        // +Y
        (
            [0.0, 1.0, 0.0],
            [
                [x0, y1, z0],
                [x0, y1, z1],
                [x1, y1, z1],
                [x1, y1, z0],
            ],
        ),
        // -Y
        (
            [0.0, -1.0, 0.0],
            [
                [x0, y0, z1],
                [x0, y0, z0],
                [x1, y0, z0],
                [x1, y0, z1],
            ],
        ),
        // +Z
        (
            [0.0, 0.0, 1.0],
            [
                [x0, y0, z1],
                [x1, y0, z1],
                [x1, y1, z1],
                [x0, y1, z1],
            ],
        ),
        // -Z
        (
            [0.0, 0.0, -1.0],
            [
                [x1, y0, z0],
                [x0, y0, z0],
                [x0, y1, z0],
                [x1, y1, z0],
            ],
        ),
    ];
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
}
