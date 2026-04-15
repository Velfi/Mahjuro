//! Procedural tessellated quad for the gameplay-scene wood table (one object in world space).
//!
//! The mesh is a flat plane in local **XY** with normal **+Z** — the Z-up world
//! table uses it directly (see [`crate::render::world_space::pixel_to_world`] + [`crate::render::table_transform::table_mesh_lay_flat`]). The tessellation is intentionally dense
//! (200×200) so the lacquered-wood vertex shader can displace the surface
//! along the grain and recompute analytical normals from a finite-difference
//! gradient — the dense grid is what turns the procedural noise into actual
//! geometric detail rather than just per-pixel shading.

use crate::render::lit_mesh::{MaterialParams, MeshCpu};
use crate::render::tile_glb::Vertex3dTex;

pub fn build_table_mesh() -> MeshCpu {
    // Dense enough that the highest-frequency noise (≈8 ring cycles per
    // local unit) is sampled ~25× per cycle. Pores are still undersampled
    // by the geometry — they live as shading-only detail in the FS.
    let segments: usize = 200;
    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity((segments + 1) * (segments + 1));
    let mut indices: Vec<u32> = Vec::new();

    for j in 0..=segments {
        for i in 0..=segments {
            // Local coords in [-0.5, +0.5] so the model matrix can scale by
            // (width, height) directly.
            let u = (i as f32) / (segments as f32);
            let v = (j as f32) / (segments as f32);
            let x = u - 0.5;
            let y = v - 0.5;
            vertices.push(Vertex3dTex {
                position: [x, y, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [u, v],
            });
        }
    }

    let stride = segments + 1;
    for j in 0..segments {
        for i in 0..segments {
            let a = (j * stride + i) as u32;
            let b = (j * stride + i + 1) as u32;
            let c = ((j + 1) * stride + i) as u32;
            let d = ((j + 1) * stride + i + 1) as u32;
            // CCW front-facing.
            indices.push(a);
            indices.push(b);
            indices.push(c);
            indices.push(c);
            indices.push(b);
            indices.push(d);
        }
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams::lacquered_wood(),
    }
}
