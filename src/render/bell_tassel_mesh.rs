//! Silk tassel under the shop leave bell — hangs from the lip along local −Z.
//! [`crate::scenes::shop::draw`] anchors [`super::shop_bell_mesh::LIP_Z`] and
//! applies a gentle wind sway on the instance rotation.

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu, push_cylinder_z};
use crate::render::tile_glb::Vertex3dTex;

fn push_sphere(
    vertices: &mut Vec<Vertex3dTex>,
    indices: &mut Vec<u32>,
    center: [f32; 3],
    radius: f32,
    lat_rings: usize,
    lon_segs: usize,
) {
    let lat_rings = lat_rings.max(3);
    let lon_segs = lon_segs.max(4);
    let row = (lon_segs + 1) as u32;
    let base = vertices.len() as u32;
    for lat in 0..=lat_rings {
        let phi = std::f32::consts::PI * (lat as f32) / (lat_rings as f32);
        let (sin_phi, cos_phi) = phi.sin_cos();
        for lon in 0..=lon_segs {
            let theta = std::f32::consts::TAU * (lon as f32) / (lon_segs as f32);
            let (sin_t, cos_t) = theta.sin_cos();
            let nx = sin_phi * cos_t;
            let ny = sin_phi * sin_t;
            let nz = cos_phi;
            vertices.push(Vertex3dTex {
                position: [
                    center[0] + radius * nx,
                    center[1] + radius * ny,
                    center[2] + radius * nz,
                ],
                normal: [nx, ny, nz],
                uv: [0.0, 0.0],
            });
        }
    }
    for lat in 0..lat_rings as u32 {
        for lon in 0..lon_segs as u32 {
            let i00 = base + lat * row + lon;
            let i01 = base + lat * row + lon + 1;
            let i10 = base + (lat + 1) * row + lon;
            let i11 = base + (lat + 1) * row + lon + 1;
            indices.extend_from_slice(&[i00, i10, i01, i01, i10, i11]);
        }
    }
}

/// Attachment point is local **+Z = 0** (lip knot); geometry extends toward **−Z**.
pub fn build_bell_tassel_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Braid / cord body.
    push_cylinder_z(&mut vertices, &mut indices, 0.0, 0.0, -0.42, 0.0, 0.034, 12);

    // Waist knot (slightly wider cylinder).
    push_cylinder_z(
        &mut vertices,
        &mut indices,
        0.0,
        0.0,
        -0.36,
        -0.28,
        0.058,
        14,
    );

    // Pom ball — weighted bundle below the knot.
    push_sphere(&mut vertices, &mut indices, [0.0, 0.0, -0.52], 0.082, 5, 10);

    // A few silk fringe strands (thin boxes), rotated around −Z.
    let fringe_z0 = -0.62_f32;
    let fringe_z1 = -0.76_f32;
    let arm = 0.045_f32;
    for k in 0..6 {
        let a = std::f32::consts::TAU * (k as f32) / 6.0;
        let (ca, sa) = (a.cos(), a.sin());
        let wx = ca * arm;
        let wy = sa * arm;
        let base = vertices.len() as u32;
        let n = [ca, sa, 0.0];
        let hw = 0.008_f32;
        let corners = [
            [wx - sa * hw, wy + ca * hw, fringe_z0],
            [wx + sa * hw, wy - ca * hw, fringe_z0],
            [wx + sa * hw, wy - ca * hw, fringe_z1],
            [wx - sa * hw, wy + ca * hw, fringe_z1],
        ];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex3dTex {
                position: *corner,
                normal: n,
                uv: *uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    for v in vertices.iter_mut() {
        v.uv = [0.0, 0.0];
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Plain,
            base_color: [0.62, 0.14, 0.18, 1.0],
            specular_strength: 0.35,
            specular_power: 48.0,
        },
    }
}
