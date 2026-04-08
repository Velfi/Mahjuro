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

    // The +Y face is the third face emitted by `push_box` (index 2). Each
    // face writes 4 vertices, so the top face occupies vertices 8..12. The
    // corner order pushed for +Y is:
    //   v8 = (x0, y1, z0)   "back-left"
    //   v9 = (x0, y1, z1)   "front-left"
    //   v10 = (x1, y1, z1)  "front-right"
    //   v11 = (x1, y1, z0)  "back-right"
    // Default UVs map u→+Z, v→-X. We want u→+X, v→+Z (text runs along the
    // long horizontal axis when viewed from the camera) — that's a 90° CCW
    // rotation around +Y of the original UV mapping.
    vertices[8].uv = [0.0, 0.0]; // back-left  → top-left of texture
    vertices[9].uv = [0.0, 1.0]; // front-left → bottom-left of texture
    vertices[10].uv = [1.0, 1.0]; // front-right → bottom-right of texture
    vertices[11].uv = [1.0, 0.0]; // back-right → top-right of texture

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
