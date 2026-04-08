//! Procedural mesh for the draws/discards remaining peg block.
//!
//! A small rectangular wooden block. The pegs themselves (which decrement as
//! plays/discards are spent) are drawn as separate small `coin_mesh` cylinder
//! instances at runtime — this mesh is just the block they sit in, so it can
//! share the lacquered-wood material and shadow-casting setup with the rest
//! of the gameplay scene's wooden chrome.
//!
//! Local space spans `-0.5..+0.5` on each axis.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

/// Build the peg block mesh: a single rectangular wooden block.
pub fn build_peg_block_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    push_box(&mut vertices, &mut indices, -0.5, 0.5, -0.5, 0.5, -0.5, 0.5);

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::LacqueredWood,
            base_color: [1.0, 1.0, 1.0, 1.0],
            specular_strength: 0.55,
            specular_power: 96.0,
        },
    }
}
