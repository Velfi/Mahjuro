//! Procedural mesh for a small carved bone tablet — used by the gameplay
//! scene's yaku selectors and (later) cascade score tokens.
//!
//! The tablet is shaped like a flattened mahjong tile: a rectangular block
//! with a slight top bevel that catches candlelight. Local space spans
//! `-0.5..+0.5` on each axis; per-instance scale supplies the actual size.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

/// Build the bone tablet mesh: a single rectangular block.
///
/// `push_box` lays out a shared `[(0,1),(1,1),(1,0),(0,0)]` UV per face. For
/// the engraved-label decal we want the text to read along the tablet's
/// **long screen-X axis** (so a multi-letter yaku name like "Toitoi" runs
/// across the wide face), but the default mapping orients the texture u-axis
/// along local Z. This rotates the +Y face's UVs by 90° CCW around UP so
/// `+u` follows local `+X` and the engraved text reads upright when viewed
/// from the camera.
pub fn build_bone_tablet_mesh() -> MeshCpu {
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
    // +Y face UVs: camera right=+X, camera up=-Z (tablet tilted Rx(-25°) so
    // local -Z is "further from player" = higher on screen).
    // push_box +Y corner order: (x0,y1,z0), (x0,y1,z1), (x1,y1,z1), (x1,y1,z0)
    //   v8  (-0.5, top, -0.5) screen-left,  screen-top    → [0, 0]
    //   v9  (-0.5, top, +0.5) screen-left,  screen-bottom → [0, 1]
    //   v10 (+0.5, top, +0.5) screen-right, screen-bottom → [1, 1]
    //   v11 (+0.5, top, -0.5) screen-right, screen-top    → [1, 0]
    vertices[8].uv = [0.0, 0.0];
    vertices[9].uv = [0.0, 1.0];
    vertices[10].uv = [1.0, 1.0];
    vertices[11].uv = [1.0, 0.0];

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
