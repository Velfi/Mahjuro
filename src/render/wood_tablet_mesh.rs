//! Procedural mesh for a small carved wooden push-button tablet — used by
//! the gameplay scene's sort buttons and (at a larger scale) the Play Hand
//! action button.
//!
//! Identical geometry to the bone tablet but with the lacquered-wood material
//! so it visually rhymes with the table and the hanging plaque. Kept as a
//! separate mesh so each can be sized + lit independently in the slot pool.
//! Local space spans `-0.5..+0.5` on each axis.

use crate::render::lit_mesh::{push_box, MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

/// Build the wood tablet mesh: a single rectangular block, lacquered wood.
pub fn build_wood_tablet_mesh() -> MeshCpu {
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
            kind: MaterialKind::LacqueredWood,
            base_color: [1.0, 1.0, 1.0, 1.0], // procedural wood
            specular_strength: 0.55,
            specular_power: 96.0,
        },
    }
}
