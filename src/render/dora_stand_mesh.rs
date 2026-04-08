//! Procedural mesh for the dora indicator's brass stand.
//!
//! A small brass plinth with a vertical back board for a single face-up tile
//! to lean against. The tile itself is drawn separately at runtime via the
//! existing tile mesh, positioned to sit in the stand's cradle.
//!
//! Local space spans `-0.5..+0.5` on each axis. The stand is wider (X) than
//! it is deep (Z), and the back board reaches to the top of local Y.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

/// Build the dora stand mesh: a flat base + a vertical back board.
pub fn build_dora_stand_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Flat base — sits on the table, holds the bottom edge of the tile.
    push_box(
        &mut vertices,
        &mut indices,
        -0.5,
        0.5,
        -0.5,
        -0.4,
        -0.5,
        0.5,
    );

    // Vertical back board — the tile leans against this.
    push_box(
        &mut vertices,
        &mut indices,
        -0.5,
        0.5,
        -0.4,
        0.5,
        -0.5,
        -0.4,
    );

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            // Polished brass; per-instance color may tint slightly per state.
            base_color: [0.95, 0.78, 0.34, 1.0],
            specular_strength: 0.75,
            specular_power: 64.0,
        },
    }
}
