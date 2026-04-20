//! Procedural mesh for brass fittings (shelf rails, trim, fittings).
//!
//! A plain unit box (±0.5 on each axis) with the Brass material wired in.
//! No heightmap, no decal. Callers scale via per-instance extents to get
//! rails, strips, or small rectangular fittings; the material honors
//! `obj.color` so the scene picks the exact brass tint.
//!
//! Intentionally minimal: this is the first primitive that opts into the
//! `Brass` material, and the first that is simple enough to not warrant
//! its own mesh builder. Once the generalized primitive system exists
//! (see TODO.md) this file should collapse into a `Shape::Cube` + a
//! `MaterialSpec::brass(..)` constructor.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

/// Build the brass-rail mesh: a plain unit box (±0.5 on each axis).
pub fn build_brass_rail_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    push_box(&mut vertices, &mut indices, -0.5, 0.5, -0.5, 0.5, -0.5, 0.5);
    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Brass,
            // Polished brass default tint; scenes may override via
            // `Object3d::color` per-instance.
            base_color: [0.86, 0.65, 0.32, 1.0],
            // Broad, soft specular lobe — museum-grade polished brass,
            // not mirror steel.
            specular_strength: 0.65,
            specular_power: 48.0,
        },
    }
}
