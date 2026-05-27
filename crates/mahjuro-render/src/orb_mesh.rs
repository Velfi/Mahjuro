//! Procedural unit sphere mesh for the material viewer scene.
//!
//! Centered at the origin with radius 0.5 so `Object3d::extents` maps directly
//! to diameter on each axis. Rendered via the `lit_mesh_pipeline`; the scene
//! supplies the material per-instance so a single shared mesh can preview
//! every `MaterialKind`.

use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::tile_glb::Vertex3dTex;

const LAT_RINGS: usize = 24;
const LON_SEGS: usize = 32;
const RADIUS: f32 = 0.5;

pub fn build_orb_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity((LAT_RINGS + 1) * (LON_SEGS + 1));
    let mut indices: Vec<u32> = Vec::with_capacity(LAT_RINGS * LON_SEGS * 6);

    for lat in 0..=LAT_RINGS {
        let phi = std::f32::consts::PI * (lat as f32) / (LAT_RINGS as f32);
        let (sin_phi, cos_phi) = phi.sin_cos();
        for lon in 0..=LON_SEGS {
            let theta = std::f32::consts::TAU * (lon as f32) / (LON_SEGS as f32);
            let (sin_t, cos_t) = theta.sin_cos();
            let x = RADIUS * sin_phi * cos_t;
            let y = RADIUS * sin_phi * sin_t;
            let z = RADIUS * cos_phi;
            vertices.push(Vertex3dTex {
                position: [x, y, z],
                normal: [sin_phi * cos_t, sin_phi * sin_t, cos_phi],
                uv: [lon as f32 / LON_SEGS as f32, lat as f32 / LAT_RINGS as f32],
                tangent: Vertex3dTex::DEFAULT_TANGENT,
                uv_emr: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
    }

    let row = (LON_SEGS + 1) as u32;
    for lat in 0..LAT_RINGS as u32 {
        for lon in 0..LON_SEGS as u32 {
            let i00 = lat * row + lon;
            let i01 = lat * row + lon + 1;
            let i10 = (lat + 1) * row + lon;
            let i11 = (lat + 1) * row + lon + 1;
            indices.extend_from_slice(&[i00, i10, i01, i01, i10, i11]);
        }
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.85, 0.85, 0.85, 1.0],
            specular_strength: 0.2,
            specular_power: 32.0,
        },
    }
}
