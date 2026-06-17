//! Procedural mesh for carved jade talisman pendants (shop + memorial).
//!
//! Each kind uses a **mask-extruded** organic silhouette (see
//! [`crate::relic_dish::build_talisman_mesh_from_rgba`]). Caps face ±Z; thickness
//! along Z.

use crate::lit_mesh::{MaterialKind, MaterialParams, MeshCpu};
use crate::relic_dish::build_talisman_mesh_from_rgba;
use crate::table_transform::euler_xyz_rad_from_deg;
use crate::tile_glb::Vertex3dTex;

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
    if let Some(cpu) = load_baked_talisman_mesh(path) {
        return Some(cpu);
    }
    let (rgba, w, h) = crate::baked_texture::load_rgba_for_cpu(path).ok()?;
    build_talisman_mesh_from_rgba(&rgba, w, h, path)
}

pub fn baked_talisman_mesh_asset_path(mask_asset_path: &str) -> String {
    let mut path = String::from("data/talisman_mesh_baked/");
    for ch in mask_asset_path.chars() {
        match ch {
            '/' | '\\' => path.push('/'),
            ':' => path.push('_'),
            _ => path.push(ch),
        }
    }
    path.push_str(".tmesh");
    path
}

pub fn load_baked_talisman_mesh(mask_asset_path: &str) -> Option<MeshCpu> {
    let path = baked_talisman_mesh_asset_path(mask_asset_path);
    let bytes = mahjuro_assets::asset_path::get_shared(&path)?;
    decode_baked_talisman_mesh(bytes.as_ref()).ok()
}

pub fn encode_baked_talisman_mesh(mesh: &MeshCpu) -> anyhow::Result<Vec<u8>> {
    let header = TalismanMeshHeader {
        magic: *TALISMAN_MESH_MAGIC,
        version: TALISMAN_MESH_VERSION,
        vertex_count: mesh.vertices.len() as u32,
        index_count: mesh.indices.len() as u32,
        material_kind: mesh.default_material.kind as u32,
        base_color: mesh.default_material.base_color,
        specular_strength: mesh.default_material.specular_strength,
        specular_power: mesh.default_material.specular_power,
    };
    let mut out = Vec::with_capacity(
        std::mem::size_of::<TalismanMeshHeader>()
            + std::mem::size_of_val(mesh.vertices.as_slice())
            + std::mem::size_of_val(mesh.indices.as_slice()),
    );
    out.extend_from_slice(bytemuck::bytes_of(&header));
    out.extend_from_slice(bytemuck::cast_slice(&mesh.vertices));
    out.extend_from_slice(bytemuck::cast_slice(&mesh.indices));
    Ok(out)
}

pub fn decode_baked_talisman_mesh(bytes: &[u8]) -> anyhow::Result<MeshCpu> {
    let header_size = std::mem::size_of::<TalismanMeshHeader>();
    anyhow::ensure!(
        bytes.len() >= header_size,
        "talisman mesh bake: file too small"
    );
    let header: &TalismanMeshHeader = bytemuck::try_from_bytes(&bytes[..header_size])
        .map_err(|e| anyhow::anyhow!("talisman mesh bake header: {e}"))?;
    anyhow::ensure!(
        header.magic == *TALISMAN_MESH_MAGIC,
        "talisman mesh bake: bad magic"
    );
    anyhow::ensure!(
        header.version == TALISMAN_MESH_VERSION,
        "talisman mesh bake: unsupported version {}",
        header.version
    );

    let vertex_bytes = (header.vertex_count as usize)
        .checked_mul(std::mem::size_of::<Vertex3dTex>())
        .ok_or_else(|| anyhow::anyhow!("talisman mesh bake: vertex length overflow"))?;
    let index_bytes = (header.index_count as usize)
        .checked_mul(std::mem::size_of::<u32>())
        .ok_or_else(|| anyhow::anyhow!("talisman mesh bake: index length overflow"))?;
    let vertex_end = header_size
        .checked_add(vertex_bytes)
        .ok_or_else(|| anyhow::anyhow!("talisman mesh bake: vertex end overflow"))?;
    let index_end = vertex_end
        .checked_add(index_bytes)
        .ok_or_else(|| anyhow::anyhow!("talisman mesh bake: index end overflow"))?;
    anyhow::ensure!(
        bytes.len() >= index_end,
        "talisman mesh bake: truncated payload"
    );

    Ok(MeshCpu {
        vertices: bytemuck::cast_slice(&bytes[header_size..vertex_end]).to_vec(),
        indices: bytemuck::cast_slice(&bytes[vertex_end..index_end]).to_vec(),
        default_material: MaterialParams {
            kind: material_kind_from_u32(header.material_kind),
            base_color: header.base_color,
            specular_strength: header.specular_strength,
            specular_power: header.specular_power,
        },
    })
}

const TALISMAN_MESH_MAGIC: &[u8; 4] = b"TMSH";
const TALISMAN_MESH_VERSION: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TalismanMeshHeader {
    magic: [u8; 4],
    version: u32,
    vertex_count: u32,
    index_count: u32,
    material_kind: u32,
    base_color: [f32; 4],
    specular_strength: f32,
    specular_power: f32,
}

fn material_kind_from_u32(kind: u32) -> MaterialKind {
    match kind {
        21 => MaterialKind::Chitin,
        _ => MaterialKind::Plain,
    }
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
