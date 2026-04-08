//! Procedural mesh for the pick-blind scene's shrine objects.
//!
//! Three shrines (Small / Big / Boss) are drawn side by side, each scaled by
//! per-instance `ShrinePlacement.extents`. The mesh is built in normalized
//! local space spanning -0.5..+0.5 on each axis so a per-instance scale
//! matrix can size it.
//!
//! Geometry (composed of axis-aligned boxes — same approach as
//! `curio_cabinet_mesh`):
//!
//! ```text
//!   +Y up
//!   ┌──────────┐               ← roof slab (wide, overhanging)
//!     ┌──────┐                  ← bowl / offering platform
//!     │ ░░░░ │
//!     │ ░░░░ │
//!     │ ░░░░ │                  ← central pillar (thin column)
//!     │ ░░░░ │
//!   ┌─┴──────┴─┐                ← plinth / base (wide, low)
//! ```

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

/// Build the shrine mesh in local space (-0.5..+0.5 on each axis).
pub fn build_shrine_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Local axis range for each piece. The mesh is symmetric in X and Z so
    // that scaling extents stretch the shrine evenly when sized up for the
    // Boss slot.

    // ── 1. Plinth: wide flat base, sits at the bottom of local space.
    // Fills almost the full XZ footprint, low Y profile.
    push_box(
        &mut vertices,
        &mut indices,
        -0.50,
        0.50, // x
        -0.50,
        -0.36, // y (bottom slab)
        -0.40,
        0.40, // z
    );
    // Plinth top step (slightly inset, slightly taller) — gives the base a
    // tiered silhouette so it reads as masonry, not just a slab.
    push_box(
        &mut vertices,
        &mut indices,
        -0.42,
        0.42,
        -0.36,
        -0.28,
        -0.34,
        0.34,
    );

    // ── 2. Central pillar: thin tall column in the middle.
    push_box(
        &mut vertices,
        &mut indices,
        -0.16,
        0.16,
        -0.28,
        0.18,
        -0.16,
        0.16,
    );

    // ── 3. Offering bowl: shallow wider box on top of the pillar. Reads
    // as the platform where the boss/blind effigy would sit.
    push_box(
        &mut vertices,
        &mut indices,
        -0.28,
        0.28,
        0.18,
        0.26,
        -0.24,
        0.24,
    );
    // Inner rim — slightly narrower and taller so the bowl reads as a
    // recessed dish, not a solid slab.
    push_box(
        &mut vertices,
        &mut indices,
        -0.24,
        0.24,
        0.26,
        0.30,
        -0.22,
        0.22,
    );

    // ── 4. Roof slab: wide overhanging eave high above the bowl. Acts as
    // the spotlight catcher — its top surface picks up the warm key light.
    push_box(
        &mut vertices,
        &mut indices,
        -0.46,
        0.46,
        0.36,
        0.44,
        -0.36,
        0.36,
    );
    // Roof ridge — small raised cap centered on top of the slab so the
    // silhouette has a peak.
    push_box(
        &mut vertices,
        &mut indices,
        -0.10,
        0.10,
        0.44,
        0.50,
        -0.36,
        0.36,
    );
    // Two thin support struts connecting the bowl rim to the roof slab,
    // front and back. They cast visible shadows from the spotlight and
    // anchor the roof visually so it doesn't appear to float.
    push_box(
        &mut vertices,
        &mut indices,
        -0.06,
        0.06,
        0.30,
        0.36,
        -0.30,
        -0.24,
    );
    push_box(
        &mut vertices,
        &mut indices,
        -0.06,
        0.06,
        0.30,
        0.36,
        0.24,
        0.30,
    );

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::LacqueredWood,
            base_color: [1.0, 1.0, 1.0, 1.0],
            specular_strength: 0.50,
            specular_power: 80.0,
        },
    }
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
            [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        ),
        // -X
        (
            [-1.0, 0.0, 0.0],
            [[x0, y0, z1], [x0, y1, z1], [x0, y1, z0], [x0, y0, z0]],
        ),
        // +Y
        (
            [0.0, 1.0, 0.0],
            [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
        ),
        // -Y
        (
            [0.0, -1.0, 0.0],
            [[x0, y0, z1], [x0, y0, z0], [x1, y0, z0], [x1, y0, z1]],
        ),
        // +Z
        (
            [0.0, 0.0, 1.0],
            [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        ),
        // -Z
        (
            [0.0, 0.0, -1.0],
            [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]],
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
