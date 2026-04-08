//! Procedural mesh for a small carved bone tablet — used by the gameplay
//! scene's yaku selectors and (later) cascade score tokens.
//!
//! The tablet is shaped like a flattened mahjong tile: a rectangular block
//! with a slight top bevel that catches candlelight. Local space spans
//! `-0.5..+0.5` on each axis; per-instance scale supplies the actual size.

use crate::render::lit_mesh::{push_box, MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

/// Build the bone tablet mesh: a single rectangular block.
pub fn build_bone_tablet_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    push_box(
        &mut vertices,
        &mut indices,
        -0.5,
        0.5,
        -0.5,
        0.5,
        -0.5,
        0.5,
    );

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            // Bone / aged ivory base color. Per-instance color overrides for
            // active vs dim states.
            base_color: [0.93, 0.89, 0.78, 1.0],
            specular_strength: 0.30,
            specular_power: 32.0,
        },
    }
}
