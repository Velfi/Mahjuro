//! Procedural mesh for a small carved wooden push-button tablet — used by
//! the gameplay scene's sort buttons and (at a larger scale) the Play Hand
//! action button.
//!
//! Identical geometry to the bone tablet but with the lacquered-wood material
//! so it visually rhymes with the table and the hanging plaque. Kept as a
//! separate mesh so each can be sized + lit independently in the slot pool.
//! Local space spans `-0.5..+0.5` on each axis.

use crate::cap_extrude::planar_y_cap_uv_xz;
use crate::lit_mesh::{Aabb, MaterialKind, MaterialParams, MeshCpu, push_box, push_quad};
use crate::tile_glb::Vertex3dTex;

/// Width of the chamfered bevel strips on the four edges of the tablet's
/// top face. In local units, where the tablet spans −0.5..+0.5 on each axis.
const BEVEL: f32 = 0.05;

/// Build the wood tablet mesh: a rectangular block with chamfered top edges,
/// lacquered wood.
pub fn build_wood_tablet_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, -0.5, 0.5, -0.5, 0.5),
    );

    // Zero out UVs on every face except +Y so the decal (transparent
    // background) only appears on the top face. push_box emits 4 verts
    // per face in order: +X(0-3), -X(4-7), +Y(8-11), -Y(12-15),
    // +Z(16-19), -Z(20-23).
    for i in (0..24).filter(|i| !(8..12).contains(i)) {
        vertices[i].uv = [0.0, 0.0];
    }
    // +Y face UVs: camera right=+X, camera up=-Z (after Rx(~140°) tilt,
    // local +Z swings downward in world space, so local -Z = screen-top).
    // push_box +Y corner order: (x0,y1,z0), (x0,y1,z1), (x1,y1,z1), (x1,y1,z0)
    //   v8  (-0.5, top, -0.5) screen-left,  screen-top    → [0, 0]
    //   v9  (-0.5, top, +0.5) screen-left,  screen-bottom → [0, 1]
    //   v10 (+0.5, top, +0.5) screen-right, screen-bottom → [1, 1]
    //   v11 (+0.5, top, -0.5) screen-right, screen-top    → [1, 0]
    // +Y face UVs via [`planar_y_cap_uv_xz`] (screen-top = local −Z → v=0).
    vertices[8].uv = planar_y_cap_uv_xz(-0.5, -0.5);
    vertices[9].uv = planar_y_cap_uv_xz(-0.5, 0.5);
    vertices[10].uv = planar_y_cap_uv_xz(0.5, 0.5);
    vertices[11].uv = planar_y_cap_uv_xz(0.5, -0.5);

    // Chamfered bevel strips around the top-face perimeter. Each strip has a
    // 45° normal blending +Y with the respective side direction, so glancing
    // light picks out a physical carved edge instead of a sharp corner. UVs
    // default to (0,0) so the decal on the +Y face doesn't bleed onto them.
    let s = std::f32::consts::FRAC_1_SQRT_2;
    push_quad(
        &mut vertices,
        &mut indices,
        // +X edge bevel: connects top face's +X edge to +X side face.
        [0.5 - BEVEL, 0.5, -0.5],
        [0.5 - BEVEL, 0.5, 0.5],
        [0.5, 0.5 - BEVEL, 0.5],
        [0.5, 0.5 - BEVEL, -0.5],
        [s, s, 0.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        // −X edge bevel.
        [-0.5, 0.5 - BEVEL, -0.5],
        [-0.5, 0.5 - BEVEL, 0.5],
        [-0.5 + BEVEL, 0.5, 0.5],
        [-0.5 + BEVEL, 0.5, -0.5],
        [-s, s, 0.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        // +Z edge bevel.
        [-0.5, 0.5 - BEVEL, 0.5],
        [0.5, 0.5 - BEVEL, 0.5],
        [0.5, 0.5, 0.5 - BEVEL],
        [-0.5, 0.5, 0.5 - BEVEL],
        [0.0, s, s],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        // −Z edge bevel.
        [-0.5, 0.5, -0.5 + BEVEL],
        [0.5, 0.5, -0.5 + BEVEL],
        [0.5, 0.5 - BEVEL, -0.5],
        [-0.5, 0.5 - BEVEL, -0.5],
        [0.0, s, -s],
    );

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
