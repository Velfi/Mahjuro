//! Procedural mesh for a hanging talisman (jade tablet) used by the shop.
//!
//! The mesh is a flat **regular octagonal** prism: eight equal sides with a
//! **flat edge toward −Y** (resting on the felt like a stop sign on its edge).
//! Flat faces point along ±Z; thickness along Z. Local space fits in the unit
//! cube so per-instance scale matrices can size each tablet.

use std::f32::consts::TAU;

use crate::render::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::render::table_transform::euler_xyz_rad_from_deg;
use crate::render::theme::color;
use crate::render::tile_glb::Vertex3dTex;

const SIDES: usize = 8;
/// Circumradius of the octagon in local XY.
const RADIUS: f32 = 0.50;
const HALF_T: f32 = 0.09;

/// Rotate the octagon so a horizontal edge sits on −Y (not a vertex).
/// `π/8` rad (`TAU/16`) puts edge midpoints at ±Y/±X; `0` puts vertices on cardinals.
const OCTAGON_ANGLE_OFFSET: f32 = TAU / 16.0;

/// Pitch (degrees) that aims carved +local Z at cameras on world −Y (table / shop / archive).
pub const TALISMAN_FACE_CAMERA_RX_DEG: f32 = 90.0;

/// Upright talisman facing the default −Y camera. `yaw_y_deg` adds a small Ry tilt (archive uses 14°).
#[inline]
pub fn talisman_face_camera_rotation(yaw_y_deg: f32) -> [f32; 3] {
    euler_xyz_rad_from_deg(TALISMAN_FACE_CAMERA_RX_DEG, yaw_y_deg, 0.0)
}

/// Face-plate UV: +local Y is the top of the heightmap (`v → 0`).
/// `v = 0.5 - y/R` maps the flat edge (−Y) to the bottom row of the texture.
#[inline]
fn talisman_face_uv(x: f32, y: f32) -> [f32; 2] {
    [x / RADIUS * 0.5 + 0.5, 0.5 - y / RADIUS * 0.5]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_uv_maps_plus_y_to_top_of_heightmap() {
        let uv_top = talisman_face_uv(0.0, RADIUS * 0.92);
        assert!(
            uv_top[1] < 0.1,
            "top of tablet should sample low v (got {})",
            uv_top[1]
        );
        let uv_bottom = talisman_face_uv(0.0, -RADIUS * 0.92);
        assert!(
            uv_bottom[1] > 0.9,
            "flat edge should sample high v (got {})",
            uv_bottom[1]
        );
    }
}

fn octagon_rim() -> [(f32, f32); SIDES] {
    std::array::from_fn(|i| {
        let theta = (i as f32) * TAU / SIDES as f32 + OCTAGON_ANGLE_OFFSET;
        (theta.cos() * RADIUS, theta.sin() * RADIUS)
    })
}

/// Build the talisman mesh (octagonal flat tablet, flat edge down).
pub fn build_talisman_mesh() -> MeshCpu {
    let mut vertices: Vec<Vertex3dTex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let rim = octagon_rim();

    // ── Front face (+Z normal): triangle fan from center vertex.
    let front_z = HALF_T;
    let front_normal = [0.0, 0.0, 1.0];
    let front_center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, 0.0, front_z],
        normal: front_normal,
        uv: [0.5, 0.5],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let front_ring_start = vertices.len() as u32;
    for &(x, y) in &rim {
        vertices.push(Vertex3dTex {
            position: [x, y, front_z],
            normal: front_normal,
            uv: talisman_face_uv(x, y),
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..SIDES {
        let i0 = front_ring_start + i as u32;
        let i1 = front_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[front_center_idx, i0, i1]);
    }

    // ── Back face (-Z normal): triangle fan, opposite winding.
    let back_z = -HALF_T;
    let back_normal = [0.0, 0.0, -1.0];
    let back_center_idx = vertices.len() as u32;
    vertices.push(Vertex3dTex {
        position: [0.0, 0.0, back_z],
        normal: back_normal,
        uv: [0.5, 0.5],
        tangent: Vertex3dTex::DEFAULT_TANGENT,
        uv_emr: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    });
    let back_ring_start = vertices.len() as u32;
    for &(x, y) in &rim {
        vertices.push(Vertex3dTex {
            position: [x, y, back_z],
            normal: back_normal,
            uv: talisman_face_uv(x, y),
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
    }
    for i in 0..SIDES {
        let i0 = back_ring_start + i as u32;
        let i1 = back_ring_start + ((i + 1) % SIDES) as u32;
        indices.extend_from_slice(&[back_center_idx, i1, i0]);
    }

    // ── Rim: eight flat side planes (one quad per edge, outward XY normal).
    for i in 0..SIDES {
        let (x0, y0) = rim[i];
        let (x1, y1) = rim[(i + 1) % SIDES];
        let mx = (x0 + x1) * 0.5;
        let my = (y0 + y1) * 0.5;
        let len = (mx * mx + my * my).sqrt().max(1e-6);
        let n = [mx / len, my / len, 0.0];
        let base = vertices.len() as u32;
        // UVs span the plane (u along edge, v front→back); never sampled for mask.
        vertices.push(Vertex3dTex {
            position: [x0, y0, front_z],
            normal: n,
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, y1, front_z],
            normal: n,
            uv: [1.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x1, y1, back_z],
            normal: n,
            uv: [1.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        vertices.push(Vertex3dTex {
            position: [x0, y0, back_z],
            normal: n,
            uv: [0.0, 1.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    MeshCpu {
        vertices,
        indices,
        default_material: MaterialParams {
            kind: MaterialKind::Chitin,
            base_color: color::PARCHMENT,
            specular_strength: 0.78,
            specular_power: 56.0,
        },
    }
}

/// Local AABB half-extents for picking / projection (regular octagon × thickness).
pub const TALISMAN_LOCAL_HALF: [f32; 3] = [RADIUS, RADIUS, HALF_T];

/// World-space `Object3d::extents` matching [`TALISMAN_LOCAL_HALF`].
pub fn talisman_object_extents(xy_extent: f32) -> [f32; 3] {
    let thickness = xy_extent * (HALF_T * 2.0) / (RADIUS * 2.0);
    [xy_extent, xy_extent, thickness]
}

/// Per-talisman material parameters. All tablets use [`MaterialKind::Chitin`] (holographic
/// foil wrapper). Per-kind spec tuning and `material_params.w` (kind index) bias the rainbow.
pub fn memorial_talisman_material(
    kind: crate::core::memorial_talisman::MemorialTalismanKind,
    base_color: [f32; 4],
) -> MaterialParams {
    let _ = kind;
    MaterialParams {
        kind: MaterialKind::Chitin,
        base_color,
        specular_strength: 0.78,
        specular_power: 56.0,
    }
}

pub fn talisman_material(
    kind: crate::core::talisman::TalismanKind,
    base_color: [f32; 4],
) -> MaterialParams {
    use crate::core::talisman::TalismanKind as T;
    let (spec_strength, spec_power) = match kind {
        T::Pearl | T::Honors | T::Wildflower => (0.78, 56.0),
        T::Gilded => (0.88, 48.0),
        T::Polychrome => (0.82, 40.0),
        T::Souzu | T::Pinzu | T::Manzu | T::Conformity => (0.80, 48.0),
    };
    MaterialParams {
        kind: MaterialKind::Chitin,
        base_color,
        specular_strength: spec_strength,
        specular_power: spec_power,
    }
}
