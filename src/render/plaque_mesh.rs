//! Procedural mesh for the gameplay scene's hanging blind/score plaque.
//!
//! A flat lacquered-wood rectangle suspended (visually) by two brass chain
//! attachment nubs at the top corners. The face decal (blind name + score
//! line) is painted on by the renderer via a per-instance albedo texture
//! sampled by the lit-mesh shader; the mesh itself is just the wood + nubs.
//!
//! Local space spans `-0.5..+0.5` on each axis so a per-instance scale matrix
//! sizes it via the `PlaquePlacement.extents`.

use crate::render::lit_mesh::{push_box, MaterialKind, MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

/// Half-thickness of the plaque slab in local units.
const HALF_Z: f32 = 0.06;
/// Width of the brass chain nubs at the top corners (local units).
const NUB_W: f32 = 0.06;
/// Height the chain nubs poke above the plaque top edge.
const NUB_H: f32 = 0.08;

/// Build the plaque mesh: a thin wood slab plus two brass chain nubs.
pub fn build_plaque_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Main wood slab.
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

    // Left chain nub.
    push_box(
        &mut vertices,
        &mut indices,
        -0.5 + 0.06,
        -0.5 + 0.06 + NUB_W,
        0.5,
        0.5 + NUB_H,
        -HALF_Z * 0.5,
        HALF_Z * 0.5,
    );
    // Right chain nub.
    push_box(
        &mut vertices,
        &mut indices,
        0.5 - 0.06 - NUB_W,
        0.5 - 0.06,
        0.5,
        0.5 + NUB_H,
        -HALF_Z * 0.5,
        HALF_Z * 0.5,
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
