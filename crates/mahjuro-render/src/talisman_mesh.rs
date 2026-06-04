//! Procedural mesh for carved jade talisman pendants (shop + memorial).
//!
//! Each kind uses a **mask-extruded** organic silhouette (see
//! [`crate::relic_dish::build_talisman_mesh_from_rgba`]). Caps face ±Z; thickness
//! along Z.

use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::relic_dish::build_talisman_mesh_from_rgba;
use crate::table_transform::euler_xyz_rad_from_deg;

/// Normalized cap half-extent after footprint scaling (see `CAP_REFERENCE_AREA`).
const CAP_HALF_EXTENT: f32 = 0.50;
/// Half-thickness of extruded pendant slabs along ±Z.
pub const TALISMAN_HALF_THICKNESS: f32 = 0.045;
const HALF_T: f32 = TALISMAN_HALF_THICKNESS;

/// Pitch (degrees) that aims carved +local Z at cameras on world −Y (table / shop / archive).
pub const TALISMAN_FACE_CAMERA_RX_DEG: f32 = 90.0;

/// Upright talisman facing the default −Y camera. `yaw_y_deg` adds a small Ry tilt (archive uses 14°).
#[inline]
pub fn talisman_face_camera_rotation(yaw_y_deg: f32) -> [f32; 3] {
    euler_xyz_rad_from_deg(TALISMAN_FACE_CAMERA_RX_DEG, yaw_y_deg, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end axis chain for mask-extruded pendants (Z-up world):
    /// image top → +local Y → v≈0; placement Rx(90°) → world +Z up, +local Z → world −Y (camera).
    #[test]
    fn extruded_mesh_axes_image_top_to_world_plus_z() {
        use crate::table_transform::rot_euler_xyz_rad;
        use glam::Vec4;

        let mesh =
            build_talisman_mesh_from_mask_asset("textures/talismans/talisman_wildflower_mask.png")
                .expect("wildflower mesh");
        let cap: Vec<_> = mesh.vertices.iter().filter(|v| v.normal[2] > 0.9).collect();
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

        let rot = rot_euler_xyz_rad(TALISMAN_FACE_CAMERA_RX_DEG.to_radians(), 0.0, 0.0);
        let world_top =
            (rot * Vec4::new(top.position[0], top.position[1], top.position[2], 1.0)).truncate();
        let world_bottom = (rot
            * Vec4::new(
                bottom.position[0],
                bottom.position[1],
                bottom.position[2],
                1.0,
            ))
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
        let cpu =
            build_talisman_mesh_from_mask_asset("textures/talismans/talisman_wildflower_mask.png")
                .expect("wildflower mask should extrude");
        assert!(!cpu.vertices.is_empty());
        assert!(!cpu.indices.is_empty());
    }
}

/// Build an extruded pendant mesh from an embedded mask asset path.
pub fn build_talisman_mesh_from_mask_asset(path: &str) -> Option<MeshCpu> {
    let file = mahjuro_assets::asset_path::get(path)?;
    let img = image::load_from_memory(&file.data).ok()?.into_rgba8();
    build_talisman_mesh_from_rgba(img.as_raw(), img.width(), img.height(), path)
}

/// Local AABB half-extents for picking / projection (normalized cap × slab thickness).
pub const TALISMAN_LOCAL_HALF: [f32; 3] = [CAP_HALF_EXTENT, CAP_HALF_EXTENT, HALF_T];

/// World-space `Object3d::extents` matching [`TALISMAN_LOCAL_HALF`].
pub fn talisman_object_extents(xy_extent: f32) -> [f32; 3] {
    let thickness = xy_extent * (HALF_T * 2.0) / (CAP_HALF_EXTENT * 2.0);
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
