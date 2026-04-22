//! Procedural mesh for the gameplay scene's hanging blind/score plaque.
//!
//! A flat lacquered-wood rectangle suspended (visually) by two brass chain
//! attachment nubs at the top corners. The face decal (blind name + score
//! line) is painted on by the renderer via a per-instance albedo texture
//! sampled by the lit-mesh shader; the mesh itself is just the wood + nubs.
//!
//! Local space spans `-0.5..+0.5` on each axis so a per-instance scale matrix
//! sizes it via the `PlaquePlacement.extents`.

use crate::render::lit_mesh::{Aabb, MaterialKind, MaterialParams, MeshCpu, push_box, push_quad};
use crate::render::tile_glb::Vertex3dTex;

/// Width of the chamfered bevel strips on the four long edges of the slab
/// (in local units, where the slab spans −0.5..+0.5).
const BEVEL: f32 = 0.04;

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
        Aabb::new(-0.5, 0.5, -0.5, 0.5, -HALF_Z, HALF_Z),
    );

    // The slab face we want the engraved-text decal to appear on is +Z
    // (the broad face that tilts toward the camera). `push_box` emits faces
    // in order +X, -X, +Y, -Y, +Z, -Z, with 4 verts per face — so the +Z
    // face occupies vertices 16..20.
    //
    // Standard Z-up world, camera right=+X, camera at large −Y looking toward +Y.
    // For the flat-lying plaque (+Z face up): screen-right = local +X, screen-top = local +Y.
    // Texture V=0 at top, V=1 at bottom (standard). U=0 at left, U=1 at right.
    // The +Z corner order pushed by `push_box` is:
    //   v16 = (x0, y0, z1)  screen-left,  screen-bottom → [0, 1]
    //   v17 = (x1, y0, z1)  screen-right, screen-bottom → [1, 1]
    //   v18 = (x1, y1, z1)  screen-right, screen-top    → [1, 0]
    //   v19 = (x0, y1, z1)  screen-left,  screen-top    → [0, 0]
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

    // Chamfered bevel strips on the four long edges of the slab front face.
    // Each strip is a quad with a 45° normal so the lit shader catches glancing
    // light and the plaque reads as having a physical beveled edge rather than
    // a knife-sharp corner.  UVs stay at (0,0) so the decal does not bleed.
    let s = std::f32::consts::FRAC_1_SQRT_2; // sin/cos 45° ≈ 0.7071
    push_quad(
        &mut vertices,
        &mut indices,
        // Top bevel: connects front-face top edge to +Y side face.
        [-0.5, 0.5 - BEVEL, HALF_Z],
        [0.5, 0.5 - BEVEL, HALF_Z],
        [0.5, 0.5, HALF_Z - BEVEL],
        [-0.5, 0.5, HALF_Z - BEVEL],
        [0.0, s, s],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        // Bottom bevel: connects front-face bottom edge to −Y side face.
        [-0.5, -0.5, HALF_Z - BEVEL],
        [0.5, -0.5, HALF_Z - BEVEL],
        [0.5, -0.5 + BEVEL, HALF_Z],
        [-0.5, -0.5 + BEVEL, HALF_Z],
        [0.0, -s, s],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        // Left bevel: connects front-face left edge to −X side face.
        [-0.5, -0.5, HALF_Z - BEVEL],
        [-0.5 + BEVEL, -0.5, HALF_Z],
        [-0.5 + BEVEL, 0.5, HALF_Z],
        [-0.5, 0.5, HALF_Z - BEVEL],
        [-s, 0.0, s],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        // Right bevel: connects front-face right edge to +X side face.
        [0.5 - BEVEL, -0.5, HALF_Z],
        [0.5, -0.5, HALF_Z - BEVEL],
        [0.5, 0.5, HALF_Z - BEVEL],
        [0.5 - BEVEL, 0.5, HALF_Z],
        [s, 0.0, s],
    );

    // Left chain nub.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(
            -0.5 + 0.06,
            -0.5 + 0.06 + NUB_W,
            0.5,
            0.5 + NUB_H,
            -HALF_Z * 0.5,
            HALF_Z * 0.5,
        ),
    );
    // Right chain nub.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(
            0.5 - 0.06 - NUB_W,
            0.5 - 0.06,
            0.5,
            0.5 + NUB_H,
            -HALF_Z * 0.5,
            HALF_Z * 0.5,
        ),
    );
    // Bevel and chain-nub vertices all sample the transparent (0,0) corner so
    // the engraved decal doesn't bleed onto them. The wood material is
    // procedural and unaffected.
    for v in vertices.iter_mut().skip(24) {
        v.uv = [0.0, 0.0];
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
