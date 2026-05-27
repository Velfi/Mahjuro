//! Dedicated 3D meshes for the post-run progression meter.
//!
//! - `build_progress_meter_rail_mesh`: faceted beveled rail profile
//! - `build_progress_meter_pip_mesh`: rounded capsule-like pip

use crate::lit_mesh::{Aabb, MaterialKind, MaterialParams, MeshCpu, push_box};
use crate::theme::color;
use crate::tile_glb::Vertex3dTex;

/// Beveled meter rail built from stacked box volumes around a shared center.
/// Unit-length on X (`-0.5..+0.5`), with an octagonal-ish cross-section.
pub fn build_progress_meter_rail_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Core body.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, -0.30, 0.30, -0.16, 0.16),
    );
    // Top and bottom chamfer wedges.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, -0.24, 0.24, 0.16, 0.34),
    );
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, -0.24, 0.24, -0.34, -0.16),
    );
    // Side chamfer wedges.
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, 0.24, 0.34, -0.16, 0.16),
    );
    push_box(
        &mut vertices,
        &mut indices,
        Aabb::new(-0.5, 0.5, -0.34, -0.24, -0.16, 0.16),
    );

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Metal,
            base_color: color::BRASS,
            specular_strength: 0.92,
            specular_power: 196.0,
        },
    }
}

/// Rounded pip mesh: unit sphere centered at origin (extent `-0.5..+0.5` on each axis).
pub fn build_progress_meter_pip_mesh() -> MeshCpu {
    const LAT_RINGS: usize = 16;
    const LON_SEGS: usize = 24;
    const RAD: f32 = 0.5;

    let mut vertices: Vec<Vertex3dTex> = Vec::with_capacity((LAT_RINGS + 1) * (LON_SEGS + 1));
    let mut indices: Vec<u32> = Vec::with_capacity(LAT_RINGS * LON_SEGS * 6);

    for lat in 0..=LAT_RINGS {
        let phi = std::f32::consts::PI * (lat as f32) / (LAT_RINGS as f32);
        let (sin_phi, cos_phi) = phi.sin_cos();
        for lon in 0..=LON_SEGS {
            let theta = std::f32::consts::TAU * (lon as f32) / (LON_SEGS as f32);
            let (sin_t, cos_t) = theta.sin_cos();

            let sx = sin_phi * cos_t;
            let sy = sin_phi * sin_t;
            let sz = cos_phi;

            let px = RAD * sx;
            let py = RAD * sy;
            let pz = RAD * sz;

            let n = glam::Vec3::new(sx, sy, sz).normalize_or_zero();
            vertices.push(Vertex3dTex {
                position: [px, py, pz],
                normal: [n.x, n.y, n.z],
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
            kind: MaterialKind::Metal,
            base_color: color::RELIC_GOLD,
            specular_strength: 0.95,
            specular_power: 164.0,
        },
    }
}
