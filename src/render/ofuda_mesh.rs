//! Procedural mesh for the gameplay scene's hanging boss-rule ofuda.
//!
//! A tall, thin paper strip with a small brass eyelet at the top. The boss
//! name and rule text are blitted onto the paper face by the renderer via a
//! per-instance albedo texture; this mesh just supplies the paper + eyelet
//! geometry.
//!
//! Local space spans `-0.5..+0.5` on each axis so a per-instance scale matrix
//! sizes it via the `OfudaPlacement.extents`.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::render::tile_glb::Vertex3dTex;

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
        -0.5,
        0.5,
        -0.5,
        0.5,
        -HALF_Z,
        HALF_Z,
    );

    // The slab face we want the title/rule decal to appear on is +Z (the
    // broad paper face that tilts toward the camera). `push_box` emits
    // faces in order +X, -X, +Y, -Y, +Z, -Z, with 4 verts per face — so
    // the +Z face occupies vertices 16..20. Reorient that face's UVs so
    // the texture's +u runs along local +X and +v runs along local -Y
    // (top of the texture sits at the top of the visible paper face).
    // The +Z corner order pushed by `push_box` is:
    //   v16 = (x0, y0, z1)  bottom-left
    //   v17 = (x1, y0, z1)  bottom-right
    //   v18 = (x1, y1, z1)  top-right
    //   v19 = (x0, y1, z1)  top-left
    vertices[16].uv = [0.0, 1.0];
    vertices[17].uv = [1.0, 1.0];
    vertices[18].uv = [1.0, 0.0];
    vertices[19].uv = [0.0, 0.0];
    // Also map the -Z broad face. The ofuda now hangs upright on the back
    // wall, and depending on camera/layout tweaks the visible side can flip;
    // keeping the ink on both paper faces avoids a blank charm if we end up
    // seeing the back. The -Z corner order pushed by `push_box` is:
    //   v20 = (x1, y0, z0)  bottom-right
    //   v21 = (x0, y0, z0)  bottom-left
    //   v22 = (x0, y1, z0)  top-left
    //   v23 = (x1, y1, z0)  top-right
    vertices[20].uv = [1.0, 1.0];
    vertices[21].uv = [0.0, 1.0];
    vertices[22].uv = [0.0, 0.0];
    vertices[23].uv = [1.0, 0.0];
    // Every other slab face samples the transparent (0,0) corner of the
    // decal so the title/rule only appears on the broad paper faces. The
    // shader composites decal as `mix(albedo, tex_rgb, tex.a)`, so alpha=0
    // leaves the paper albedo untouched on the edges.
    for i in 0..16 {
        vertices[i].uv = [0.0, 0.0];
    }

    // Brass eyelet centered at the top edge.
    let eyelet_base = vertices.len();
    push_box(
        &mut vertices,
        &mut indices,
        -EYELET_W * 0.5,
        EYELET_W * 0.5,
        0.5,
        0.5 + EYELET_H,
        -HALF_Z * 1.5,
        HALF_Z * 1.5,
    );
    // Eyelet samples the transparent corner so the title/rule decal stays
    // pinned to the paper face only.
    for i in eyelet_base..vertices.len() {
        vertices[i].uv = [0.0, 0.0];
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
