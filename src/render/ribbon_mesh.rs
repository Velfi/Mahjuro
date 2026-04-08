//! Procedural mesh for a hanging silk ribbon, used by the shop scene.
//!
//! The mesh is a thin vertical strip subdivided into ~12 segments along its
//! length so the lit-mesh shader's per-fragment lighting reads as a smooth
//! gradient down the ribbon. Local extents:
//!
//! - x ∈ [-0.5, 0.5] — width
//! - y ∈ [-1.0, 0.0] — hangs *down* from the anchor at local origin
//! - z ≈ ±0.05      — slight thickness so a back-face exists
//!
//! UVs run 0→1 along y for future texturing/animation.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

const SEGMENTS: usize = 12;
const HALF_THICKNESS: f32 = 0.05;

/// Build a hanging-ribbon mesh. Front face (toward +Z) has many segments
/// for smooth lighting; back face is a single quad. Two side strips close
/// the seam so the ribbon has a tiny visible edge.
pub fn build_ribbon_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Helper: push a quad as two triangles given four corner vertices
    // already appended to `vertices`, returning the next base index.
    let push_quad = |indices: &mut Vec<u32>, base: u32| {
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    // ── Front face (+Z normal), subdivided into SEGMENTS quads top→bottom.
    let front_normal = [0.0, 0.0, 1.0];
    for s in 0..SEGMENTS {
        let v0 = s as f32 / SEGMENTS as f32;
        let v1 = (s + 1) as f32 / SEGMENTS as f32;
        let y0 = -v0; // y top of this segment (less negative)
        let y1 = -v1; // y bottom of this segment (more negative)
        let base = vertices.len() as u32;
        // Order: top-left, top-right, bottom-right, bottom-left.
        vertices.push(Vertex3dTex {
            position: [-0.5, y0, HALF_THICKNESS],
            normal: front_normal,
            uv: [0.0, v0],
        });
        vertices.push(Vertex3dTex {
            position: [0.5, y0, HALF_THICKNESS],
            normal: front_normal,
            uv: [1.0, v0],
        });
        vertices.push(Vertex3dTex {
            position: [0.5, y1, HALF_THICKNESS],
            normal: front_normal,
            uv: [1.0, v1],
        });
        vertices.push(Vertex3dTex {
            position: [-0.5, y1, HALF_THICKNESS],
            normal: front_normal,
            uv: [0.0, v1],
        });
        push_quad(&mut indices, base);
    }

    // ── Back face (-Z normal), single quad.
    let back_normal = [0.0, 0.0, -1.0];
    let base = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.5, 0.0, -HALF_THICKNESS],
        normal: back_normal,
        uv: [0.0, 0.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, 0.0, -HALF_THICKNESS],
        normal: back_normal,
        uv: [1.0, 0.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, -1.0, -HALF_THICKNESS],
        normal: back_normal,
        uv: [1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [0.5, -1.0, -HALF_THICKNESS],
        normal: back_normal,
        uv: [0.0, 1.0],
    });
    push_quad(&mut indices, base);

    // ── Left edge strip (-X normal).
    let left_normal = [-1.0, 0.0, 0.0];
    let base = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [-0.5, 0.0, -HALF_THICKNESS],
        normal: left_normal,
        uv: [0.0, 0.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, 0.0, HALF_THICKNESS],
        normal: left_normal,
        uv: [1.0, 0.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, -1.0, HALF_THICKNESS],
        normal: left_normal,
        uv: [1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [-0.5, -1.0, -HALF_THICKNESS],
        normal: left_normal,
        uv: [0.0, 1.0],
    });
    push_quad(&mut indices, base);

    // ── Right edge strip (+X normal).
    let right_normal = [1.0, 0.0, 0.0];
    let base = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.5, 0.0, HALF_THICKNESS],
        normal: right_normal,
        uv: [0.0, 0.0],
    });
    vertices.push(Vertex3dTex {
        position: [0.5, 0.0, -HALF_THICKNESS],
        normal: right_normal,
        uv: [1.0, 0.0],
    });
    vertices.push(Vertex3dTex {
        position: [0.5, -1.0, -HALF_THICKNESS],
        normal: right_normal,
        uv: [1.0, 1.0],
    });
    vertices.push(Vertex3dTex {
        position: [0.5, -1.0, HALF_THICKNESS],
        normal: right_normal,
        uv: [0.0, 1.0],
    });
    push_quad(&mut indices, base);

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            // Default base color is overridden per-instance from the
            // ZodiacRibbonPlacement; this fallback is a soft cream.
            base_color: [0.92, 0.86, 0.72, 1.0],
            specular_strength: 0.25,
            specular_power: 16.0,
        },
    }
}
