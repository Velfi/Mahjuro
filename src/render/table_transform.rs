//! Fixed-axis rotations for meshes in [`crate::render::world_space`].
//!
//! # World space (single convention)
//!
//! Everything ultimately uses [`crate::render::world_space`]: right-handed **Z-up**,
//! table in the **XY** plane, [`crate::render::world_space::pixel_to_world`] for layout → world.
//! Helpers here map **asset-local** axes into that frame — prefer composing these instead of ad‑hoc
//! basis matrices in renderers.
//!
//! # Units
//!
//! - **Degrees** — parameters and [`crate::render::draw_cmd`] fields whose names end in `_deg`.
//! - **Radians** — `*_rad` helpers, hand/showcase tile euler triples, and anything
//!   passed to [`rot_euler_xyz_rad`].
//!
//! # Matrices
//!
//! Column vectors: `p' = M * p`. For `R3 * R2 * R1 * p`, apply **R1** to **p** first, then R2, then R3.
//!
//! # Fixed-axis rotations (degrees) + world-axis (radians)
//!
//! Table props use [`rot_fixed_axes_deg`] — **`Rx * Ry * Rz`** in degrees, matching
//! [`rot_euler_xyz_rad`] / `glam::EulerRot::XYZ` (**pitch X**, **yaw Y**, **roll Z**).
//! When you already have a pure rotation matrix, [`rot_fixed_axes_deg_matrix`] maps it
//! to that same convention.
//!
//! | Helper | Notes |
//! |--------|--------|
//! | [`rot_euler_xyz_rad`] | `glam::EulerRot::XYZ` — hand tiles + showcase tiles |
//! | [`mesh_y_thickness_along_local_y_to_z_up`] | `Rx(+π/2)` | Coin/candle meshes with thickness on **local +Y** → **world +Z** (sits on XY felt) |
//! | [`tile_mesh_local_to_world`] | fixed permutation | Mahjong tile GLB local → world (see doc) |
//! | [`translate_rot_scale`] | `T * R * S` | World-space instance pose |
//! | [`ribbon_submesh`] | optional local **Y** offset + non-uniform scale | Zodiac ribbon mesh (centroid at origin; after anchor [`translate_rot_scale`]) |
//!
use glam::{EulerRot, Mat4, Vec3, Vec4};

/// Table procedural mesh is already in **XY** with normal **+Z**; no rotation.
#[inline]
pub fn table_mesh_lay_flat() -> Mat4 {
    Mat4::IDENTITY
}

/// Mahjong tile mesh from [`crate::render::tile_glb`] after `normalize_mesh` (which may rotate
/// vertices so thickness lands on **local +Y**).
///
/// **Source asset:** **Z-up Blender** + glTF 2.0 export. Expected layout **after** `normalize_mesh`:
/// **local +X** = long face axis, **local +Y** = thickness / face normal, **local +Z** = short axis.
///
/// Maps into world Z-up (standard conventions: +X right, +Y away from player, +Z up):
/// **local X → world +Y** (along-table depth axis), **local Y → world +Z** (up from the felt),
/// **local Z → world +X** (left / right). Compose with [`translate_rot_scale`] and per-tile Euler
/// from scenes.
#[inline]
pub fn tile_mesh_local_to_world() -> Mat4 {
    Mat4::from_cols(
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 0.0),
        Vec4::new(1.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    )
}

/// Revolved / cylinder meshes with extent along **local +Y** (coin thickness, candle, peg shaft).
/// Right-handed **+π/2** about **+X** sends mesh **+Y** to **world +Z** so the asset sits on the
/// **XY** felt with vertical extent along **+Z**.
#[inline]
pub fn mesh_y_thickness_along_local_y_to_z_up() -> Mat4 {
    Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
}

/// Same as `Mat4::from_euler(EulerRot::XYZ, …)` — hand tiles + showcase tiles.
#[inline]
pub fn rot_euler_xyz_rad(rx: f32, ry: f32, rz: f32) -> Mat4 {
    Mat4::from_euler(EulerRot::XYZ, rx, ry, rz)
}

/// **`Rx * Ry * Rz`** in degrees — same convention as [`rot_euler_xyz_rad`] (`EulerRot::XYZ`).
#[inline]
pub fn rot_fixed_axes_deg(pitch_x_deg: f32, yaw_y_deg: f32, roll_z_deg: f32) -> Mat4 {
    rot_euler_xyz_rad(
        pitch_x_deg.to_radians(),
        yaw_y_deg.to_radians(),
        roll_z_deg.to_radians(),
    )
}

/// Euler **XYZ** radians extracted from the rotation part of `m` (same convention as [`rot_euler_xyz_rad`]).
#[inline]
pub fn mat4_to_euler_xyz_rad(m: Mat4) -> [f32; 3] {
    let (_, q, _) = m.to_scale_rotation_translation();
    let (xr, yr, zr) = q.to_euler(EulerRot::XYZ);
    [xr, yr, zr]
}

/// **`Rx·Ry·Rz`** from degrees — convenience for filling [`crate::render::draw_cmd::Object3d::rotation`].
#[inline]
pub fn euler_xyz_rad_from_deg(rx_deg: f32, ry_deg: f32, rz_deg: f32) -> [f32; 3] {
    [
        rx_deg.to_radians(),
        ry_deg.to_radians(),
        rz_deg.to_radians(),
    ]
}

/// Same rotation as `m`, re-encoded through Euler XYZ (see [`mat4_to_euler_xyz_rad`]).
#[inline]
pub fn rot_fixed_axes_deg_matrix(m: Mat4) -> Mat4 {
    let e = mat4_to_euler_xyz_rad(m);
    rot_euler_xyz_rad(e[0], e[1], e[2])
}

/// Extruded score popups: yaw around **Z** then pitch around **X** (radians).
#[inline]
pub fn score_popup_glyph_rot_rad(rotation_z: f32, rotation_x: f32) -> Mat4 {
    Mat4::from_rotation_z(rotation_z) * Mat4::from_rotation_x(rotation_x)
}

/// Apply **`Rz * Ry * Rx`** placement degrees to an existing model matrix (translation preserved).
#[inline]
pub fn apply_rotation_deg_to_model(model: Mat4, rot_deg: [f32; 3]) -> Mat4 {
    if rot_deg[0] == 0.0 && rot_deg[1] == 0.0 && rot_deg[2] == 0.0 {
        return model;
    }
    let r = Mat4::from_rotation_z(rot_deg[2].to_radians())
        * Mat4::from_rotation_y(rot_deg[1].to_radians())
        * Mat4::from_rotation_x(rot_deg[0].to_radians());
    let t = model.w_axis.truncate();
    let nx = r.transform_vector3(model.x_axis.truncate());
    let ny = r.transform_vector3(model.y_axis.truncate());
    let nz = r.transform_vector3(model.z_axis.truncate());
    Mat4::from_cols(
        nx.extend(0.0),
        ny.extend(0.0),
        nz.extend(0.0),
        t.extend(1.0),
    )
}

/// Compose a base orientation matrix with placement rotation degrees (`R_place * R_base`).
#[inline]
pub fn compose_rotation_euler(base: Mat4, rot_deg: [f32; 3]) -> [f32; 3] {
    mat4_to_euler_xyz_rad(apply_rotation_deg_to_model(base, rot_deg))
}

/// `Translation * rotation * scale` — standard lit-mesh instance pose on the table.
/// Use [`Mat4::IDENTITY`] for rotation when the mesh has no orientation (axis-aligned boxes).
/// For ribbon anchor poses with no per-axis scale yet, pass **`Vec3::splat(1.0)`**
/// so the result is **`T * R`** only.
#[inline]
pub fn translate_rot_scale(center: Vec3, rotation: Mat4, scale: Vec3) -> Mat4 {
    Mat4::from_translation(center) * rotation * Mat4::from_scale(scale)
}

/// Zodiac ribbon submesh after the anchor [`translate_rot_scale`]: optional offset along
/// **local Y**, then non-uniform scale. Pass `local_offset_y = 0` for the single-segment
/// ribbon mesh (origin at centroid); the helper still takes an offset for per-segment reuse.
#[inline]
pub fn ribbon_submesh(parent: Mat4, local_offset_y: f32, scale: Vec3) -> Mat4 {
    parent * Mat4::from_translation(Vec3::new(0.0, local_offset_y, 0.0)) * Mat4::from_scale(scale)
}
