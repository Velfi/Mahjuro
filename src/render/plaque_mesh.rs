//! Procedural mesh for the gameplay scene's hanging blind/score plaque.
//!
//! A flat lacquered-wood rectangle suspended (visually) by two brass chain
//! attachment nubs at the top corners. The face decal (blind name + score
//! line) is painted on by the renderer via a per-instance albedo texture
//! sampled by the lit-mesh shader; the mesh itself is just the wood + nubs.
//!
//! Local space spans `-0.5..+0.5` on each axis so a per-instance scale matrix
//! sizes it via the `PlaquePlacement.extents`.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
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

    // The slab face we want the engraved-text decal to appear on is +Z
    // (the broad face that tilts toward the camera). `push_box` emits faces
    // in order +X, -X, +Y, -Y, +Z, -Z, with 4 verts per face — so the +Z
    // face occupies vertices 16..20. Reorient that face's UVs so the
    // texture's +u runs along local +X (decal reads landscape across the
    // long axis) and +v runs along local -Y (top of the texture sits at
    // the top of the visible plaque face). The +Z corner order pushed by
    // `push_box` is:
    //   v16 = (x0, y0, z1)  bottom-left
    //   v17 = (x1, y0, z1)  bottom-right
    //   v18 = (x1, y1, z1)  top-right
    //   v19 = (x0, y1, z1)  top-left
    vertices[16].uv = [0.0, 1.0];
    vertices[17].uv = [1.0, 1.0];
    vertices[18].uv = [1.0, 0.0];
    vertices[19].uv = [0.0, 0.0];
    // Every other slab face samples the transparent (0,0) corner of the
    // decal so the engraved label *only* appears on the front face. The
    // procedural lacquered-wood material still renders normally because
    // the shader composites the decal as `mix(albedo, tex_rgb, tex.a)` —
    // alpha=0 leaves the wood albedo untouched.
    for i in (0..16).chain(20..24) {
        vertices[i].uv = [0.0, 0.0];
    }

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
    // Chain-nub vertices (24..72) all sample the transparent corner so
    // the engraved decal doesn't bleed onto them. The wood material is
    // procedural and unaffected.
    for i in 24..vertices.len() {
        vertices[i].uv = [0.0, 0.0];
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            // `LacqueredWoodFlat` (not `LacqueredWood`): the table-tuned
            // wood vertex displacement amplitude (1.6 world units) is
            // larger than the plaque slab's world-Z thickness (~0.96),
            // so the regular `LacqueredWood` would shove the front face
            // vertices forward and backward through the slab and shade
            // them as ghost rectangles on the face. The flat variant
            // uses the same wood albedo branch but skips displacement.
            kind: MaterialKind::LacqueredWoodFlat,
            base_color: [1.0, 1.0, 1.0, 1.0], // procedural wood
            specular_strength: 0.55,
            specular_power: 96.0,
        },
    }
}
