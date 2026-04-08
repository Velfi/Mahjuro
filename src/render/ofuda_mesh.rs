//! Procedural mesh for the gameplay scene's hanging boss-rule ofuda.
//!
//! A tall, thin paper strip with a small brass eyelet at the top. The boss
//! name and rule text are blitted onto the paper face by the renderer via a
//! per-instance albedo texture; this mesh just supplies the paper + eyelet
//! geometry.
//!
//! Local space spans `-0.5..+0.5` on each axis so a per-instance scale matrix
//! sizes it via the `OfudaPlacement.extents`.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

/// Half-thickness of the paper slab.
const HALF_Z: f32 = 0.018;
/// Eyelet width.
const EYELET_W: f32 = 0.10;
/// Eyelet height (poking above the paper top edge).
const EYELET_H: f32 = 0.07;

/// Build the ofuda mesh: a thin paper slab plus a brass eyelet at the top.
pub fn build_ofuda_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Paper slab.
    push_box(
        &mut vertices,
        &mut indices,
        -0.5,
        0.5,
        -0.5,
        0.5,
        -HALF_Z,
        HALF_Z,
    );

    // Brass eyelet centered at the top edge.
    push_box(
        &mut vertices,
        &mut indices,
        -EYELET_W * 0.5,
        EYELET_W * 0.5,
        0.5,
        0.5 + EYELET_H,
        -HALF_Z * 1.5,
        HALF_Z * 1.5,
    );

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            // Default base color: warm paper white. Per-instance color
            // overrides this for distressed / aged variants per boss.
            base_color: [0.94, 0.91, 0.82, 1.0],
            specular_strength: 0.05,
            specular_power: 8.0,
        },
    }
}
