//! Procedural mesh for the gameplay scene's hanging boss-rule ofuda.
//!
//! A tall, thin paper strip with a small brass eyelet at the top. The boss
//! name and rule text are blitted onto the paper face by the renderer via a
//! per-instance albedo texture; this mesh just supplies the paper + eyelet
//! geometry.
//!
//! Local space spans `-0.5..+0.5` on each axis so a per-instance scale matrix
//! sizes it via the `OfudaPlacement.extents`.

use crate::cap_extrude::parametric_cap_uv;
use crate::lit_mesh::{Aabb, MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::tile_glb::Vertex3dTex;

/// Half-thickness of the paper slab.
const HALF_Z: f32 = 0.018;
/// Eyelet width.
const EYELET_W: f32 = 0.10;
/// Eyelet height (poking above the paper top edge).
const EYELET_H: f32 = 0.07;

/// Build the ofuda mesh: a thin paper slab plus a brass eyelet at the top.
pub fn build_ofuda_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Paper slab.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, -0.5, 0.5, -HALF_Z, HALF_Z),
    );

    // The slab face we want the title/rule decal to appear on is +Z (the
    // broad paper face). Standard Z-up, camera right=+X, camera at large -Y.
    // For the flat-lying ofuda (+Z face up): screen-right = local +X, screen-top = local +Y.
    // Texture V=0 at top (local +Y), V=1 at bottom (local -Y). U=0 at left, U=1 at right.
    // The +Z corner order pushed by `push_box` is:
    //   v16 = (x0, y0, z1)  screen-left,  screen-bottom → [0, 1]
    //   v17 = (x1, y0, z1)  screen-right, screen-bottom → [1, 1]
    //   v18 = (x1, y1, z1)  screen-right, screen-top    → [1, 0]
    //   v19 = (x0, y1, z1)  screen-left,  screen-top    → [0, 0]
    // +Z paper face: [`parametric_cap_uv`] (+Y screen-top → low v).
    vertices[16].uv = parametric_cap_uv(-0.5, -0.5);
    vertices[17].uv = parametric_cap_uv(0.5, -0.5);
    vertices[18].uv = parametric_cap_uv(0.5, 0.5);
    vertices[19].uv = parametric_cap_uv(-0.5, 0.5);
    // Every non-front face (including the -Z back) samples the transparent
    // (0,0) corner of the decal so the title/rule only appears on the +Z
    // paper face. The shader composites decal as `mix(albedo, tex_rgb,
    // tex.a)`, so alpha=0 leaves the paper albedo untouched. Mapping the
    // back face to the decal (even mirrored) bled through as reversed text
    // whenever camera pitch let the -Z face win the depth test.
    for v in vertices.iter_mut().take(16) {
        v.uv = [0.0, 0.0];
    }
    for v in vertices.iter_mut().take(24).skip(20) {
        v.uv = [0.0, 0.0];
    }

    // Brass eyelet centered at the top edge.
    let eyelet_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(
            -EYELET_W * 0.5,
            EYELET_W * 0.5,
            0.5,
            0.5 + EYELET_H,
            -HALF_Z * 1.5,
            HALF_Z * 1.5,
        ),
    );
    // Eyelet samples the transparent corner so the title/rule decal stays
    // pinned to the paper face only.
    for v in vertices.iter_mut().skip(eyelet_base) {
        v.uv = [0.0, 0.0];
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            // Default base color: warm paper white. Per-instance color
            // overrides this for distressed / aged variants per boss.
            base_color: [0.94, 0.91, 0.82, 1.0],
            specular_strength: 0.05,
            specular_power: 8.0,
        },
    }
}
