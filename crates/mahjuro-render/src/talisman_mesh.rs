//! Procedural mesh for carved jade talisman pendants (shop + memorial).
//!
//! Each kind uses a **mask-extruded** organic silhouette (see
//! [`crate::relic_dish::build_talisman_mesh_from_rgba`]). Caps face ±Z; thickness
//! along Z. A legacy octagonal prism ([`build_talisman_mesh`]) remains as fallback
//! until all per-kind masks are available.

use std::f32::consts::TAU;

use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::relic_dish::build_talisman_mesh_from_rgba;
use crate::table_transform::euler_xyz_rad_from_deg;
use crate::theme::color;
use crate::tile_glb::Vertex3dTex;

const SIDES: usize = 8;
/// Circumradius of the octagon in local XY (fallback mesh + extent scaling).
const RADIUS: f32 = 0.50;
/// Half-thickness of extruded pendant slabs along ±Z.
pub const TALISMAN_HALF_THICKNESS: f32 = 0.045;
const HALF_T: f32 = TALISMAN_HALF_THICKNESS;

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

use crate::cap_extrude::parametric_cap_uv;

/// Face-plate UV for the legacy octagon mesh (cap coords ±RADIUS → parametric ±0.5).
#[inline]
fn talisman_face_uv(x: f32, y: f32) -> [f32; 2] {
    parametric_cap_uv(x / (2.0 * RADIUS), y / (2.0 * RADIUS))
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

    /// End-to-end axis chain for mask-extruded pendants (Z-up world):
    /// image top → +local Y → v≈0; placement Rx(90°) → world +Z up, +local Z → world −Y (camera).
    #[test]
    fn extruded_mesh_axes_image_top_to_world_plus_z() {
        use crate::table_transform::rot_euler_xyz_rad;
        use glam::Vec4;

        let mesh = build_talisman_mesh_from_mask_asset("textures/talismans/talisman_wildflower_mask.png")
            .expect("wildflower mesh");
        let cap: Vec<_> = mesh
            .vertices
            .iter()
            .filter(|v| v.normal[2] > 0.9)
            .collect();
        assert!(!cap.is_empty(), "expected +Z front cap");

        let top = cap
            .iter()
            .max_by(|a, b| a.position[1].partial_cmp(&b.position[1]).unwrap())
            .unwrap();
        let bottom = cap
            .iter()
            .min_by(|a, b| a.position[1].partial_cmp(&b.position[1]).unwrap())
            .unwrap();

        assert!(
            top.uv[1] < bottom.uv[1],
            "+local Y should map to lower texture v (top v={:.3}, bottom v={:.3})",
            top.uv[1],
            bottom.uv[1],
        );

        let rot = rot_euler_xyz_rad(
            TALISMAN_FACE_CAMERA_RX_DEG.to_radians(),
            0.0,
            0.0,
        );
        let world_top = (rot * Vec4::new(top.position[0], top.position[1], top.position[2], 1.0)).truncate();
        let world_bottom =
            (rot * Vec4::new(bottom.position[0], bottom.position[1], bottom.position[2], 1.0))
                .truncate();

        assert!(
            world_top.z > world_bottom.z + 1e-4,
            "image top (+local Y) should land world +Z after Rx(90°) (top z={:.4}, bottom z={:.4})",
            world_top.z,
            world_bottom.z,
        );
        assert!(
            world_top.y < -1e-4 && world_bottom.y < -1e-4,
            "carved face (+local Z) should point world −Y toward default camera (top y={:.4}, bottom y={:.4})",
            world_top.y,
            world_bottom.y,
        );

        let face = (rot * Vec4::new(0.0, 0.0, 1.0, 0.0)).truncate();
        assert!(
            face.y < -0.9 && face.z.abs() < 0.1,
            "+local Z should map to world −Y (got {:?})",
            face
        );
        let up = (rot * Vec4::new(0.0, 1.0, 0.0, 0.0)).truncate();
        assert!(
            up.z > 0.9 && up.y.abs() < 0.1,
            "+local Y should map to world +Z (got {:?})",
            up
        );
    }

    #[test]
    fn mask_asset_extrudes_pendant_mesh() {
        let cpu = build_talisman_mesh_from_mask_asset("textures/talismans/talisman_wildflower_mask.png")
            .expect("wildflower mask should extrude");
        assert!(!cpu.vertices.is_empty());
        assert!(!cpu.indices.is_empty());
    }
}

fn octagon_rim() -> [(f32, f32); SIDES] {
    std::array::from_fn(|i| {
        let theta = (i as f32) * TAU / SIDES as f32 + OCTAGON_ANGLE_OFFSET;
        (theta.cos() * RADIUS, theta.sin() * RADIUS)
    })
}

/// Build an extruded pendant mesh from an embedded mask asset path.
pub fn build_talisman_mesh_from_mask_asset(path: &str) -> Option<MeshCpu> {
    let file = mahjuro_assets::asset_path::get(path)?;
    let img = image::load_from_memory(&file.data).ok()?.into_rgba8();
    build_talisman_mesh_from_rgba(
        img.as_raw(),
        img.width(),
        img.height(),
        path,
    )
}

/// Build the legacy octagonal fallback mesh (flat edge down).
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

/// Per-talisman material parameters. All tablets use [`MaterialKind::Chitin`] (nacreous
/// chitin). Shop tablets are lustrous; memorial uses lower spec (shader reads `w >= 128`).
/// Per-kind spec tuning and `material_params.w` (kind index) bias the nacre phase.
pub fn memorial_talisman_material(
    kind: mahjuro_core::core::memorial_talisman::MemorialTalismanKind,
    base_color: [f32; 4],
) -> MaterialParams {
    let _ = kind;
    MaterialParams {
        kind: MaterialKind::Chitin,
        base_color,
        specular_strength: 0.55,
        specular_power: 32.0,
    }
}

pub fn talisman_material(
    kind: mahjuro_core::core::talisman::TalismanKind,
    base_color: [f32; 4],
) -> MaterialParams {
    use mahjuro_core::core::talisman::TalismanKind as T;
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
