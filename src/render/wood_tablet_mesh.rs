//! Procedural mesh for a small carved wooden push-button tablet — used by
//! the gameplay scene's sort buttons and (at a larger scale) the Play Hand
//! action button.
//!
//! Identical geometry to the bone tablet but with the lacquered-wood material
//! so it visually rhymes with the table and the hanging plaque. Kept as a
//! separate mesh so each can be sized + lit independently in the slot pool.
//! Local space spans `-0.5..+0.5` on each axis.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

/// Build the wood tablet mesh: a single rectangular block, lacquered wood.
pub fn build_wood_tablet_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    push_box(&mut vertices, &mut indices, -0.5, 0.5, -0.5, 0.5, -0.5, 0.5);

    // Zero out UVs on every face except +Y so the decal (transparent
    // background) only appears on the top face. push_box emits 4 verts
    // per face in order: +X(0-3), -X(4-7), +Y(8-11), -Y(12-15),
    // +Z(16-19), -Z(20-23).
    for i in (0..24).filter(|i| !(8..12).contains(i)) {
        vertices[i].uv = [0.0, 0.0];
    }
    // Rotate the +Y face UVs 90° CCW around UP so the engraved label
    // texture (landscape) reads along the long screen-X axis instead
    // of the short local-Z axis.
    vertices[8].uv = [0.0, 0.0];
    vertices[9].uv = [0.0, 1.0];
    vertices[10].uv = [1.0, 1.0];
    vertices[11].uv = [1.0, 0.0];

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::LacqueredWoodFlat,
            base_color: [1.0, 1.0, 1.0, 1.0], // procedural wood
            specular_strength: 0.55,
            specular_power: 96.0,
        },
    }
}
