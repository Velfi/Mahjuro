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
//! # Which helper for which content
//!
//! | Helper | Composition | Used for |
//! |--------|-------------|----------|
//! | [`rot_rz_ry_rx_deg`] | `Rz * Ry * Rx` | Talismans, zodiac ribbons (degrees) |
//! | [`rot_rx_ry_rz_deg`] | `Rx * Ry * Rz` | Relic showcase mesh (collection / modal) |
//! | [`rot_rx_rz_deg`] | `Rx * Rz` | Relic row on dish (lean + wobble) — **same** in lit + shadow pass |
//! | [`rot_ry_rx_deg`] | `Ry * Rx` | Foil packs, plaques, ofuda |
//! | [`rot_rz_rx_deg`] | `Rz * Rx` | Wood / yaku HUD tablets, bronze mirror |
//! | [`rot_rz_rx_ry_deg`] | `Rz * Rx * Ry` | General [`crate::render::draw_cmd::Object3d`] placement |
//! | [`rot_euler_xyz_rad`] | `glam::EulerRot::XYZ` | Hand tiles + showcase tiles |
//! | [`rot_z_rad`] / [`rot_x_rad`] | single axis | Spin / pitch in Z-up world (e.g. coins: `rot_z` = yaw on table) |
//! | [`mesh_y_thickness_along_local_y_to_z_up`] | `Rx(+π/2)` | Coin/candle meshes with thickness on **local +Y** → **world +Z** (sits on XY felt) |
//! | [`tile_mesh_local_to_world`] | fixed permutation | Mahjong tile GLB local → world (see doc) |
//! | [`translate_rot_scale`] | `T * R * S` | World-space instance pose |
//! | [`ribbon_submesh`] | ribbon segments along local **-Y** | Zodiac ribbon caps/mid (after anchor [`translate_rot_scale`]) |
//!
//! Do **not** hand-compose the same pose with a different multiply order — pick the row that
//! matches the [`crate::render::draw_cmd`] struct you are filling.

use glam::{Mat4, Vec3, Vec4};

// ── Degrees (table props) ─────────────────────────────────────────────

/// `Rz(roll) * Ry(yaw) * Rx(pitch)` in degrees — talisman, ribbon, mirror-style chains.
#[inline]
pub fn rot_rz_ry_rx_deg(pitch_x_deg: f32, yaw_y_deg: f32, roll_z_deg: f32) -> Mat4 {
    Mat4::from_rotation_z(roll_z_deg.to_radians())
        * Mat4::from_rotation_y(yaw_y_deg.to_radians())
        * Mat4::from_rotation_x(pitch_x_deg.to_radians())
}

/// `Rx(pitch) * Ry(yaw) * Rz(roll)` in degrees — relic showcase viewers.
#[inline]
pub fn rot_rx_ry_rz_deg(pitch_x_deg: f32, yaw_y_deg: f32, roll_z_deg: f32) -> Mat4 {
    Mat4::from_rotation_x(pitch_x_deg.to_radians())
        * Mat4::from_rotation_y(yaw_y_deg.to_radians())
        * Mat4::from_rotation_z(roll_z_deg.to_radians())
}

/// `Rx(pitch) * Rz(roll)` in degrees — relic row on the dish (lean + wiggle).
#[inline]
pub fn rot_rx_rz_deg(pitch_x_deg: f32, roll_z_deg: f32) -> Mat4 {
    Mat4::from_rotation_x(pitch_x_deg.to_radians()) * Mat4::from_rotation_z(roll_z_deg.to_radians())
}

/// `Ry(yaw) * Rx(pitch)` in degrees — foil packs, plaques, ofuda (pitch first, then yaw).
#[inline]
pub fn rot_ry_rx_deg(pitch_x_deg: f32, yaw_y_deg: f32) -> Mat4 {
    Mat4::from_rotation_y(yaw_y_deg.to_radians()) * Mat4::from_rotation_x(pitch_x_deg.to_radians())
}

/// `Rz(roll) * Rx(pitch)` in degrees — wood / yaku tablets, bronze mirror (fixed camera tilt + z-wiggle).
#[inline]
pub fn rot_rz_rx_deg(pitch_x_deg: f32, roll_z_deg: f32) -> Mat4 {
    Mat4::from_rotation_z(roll_z_deg.to_radians()) * Mat4::from_rotation_x(pitch_x_deg.to_radians())
}

// ── Radians (gameplay / animation) ────────────────────────────────────

/// Yaw around **world +Z** (vertical) — on-table spin for coins, bars, book, etc.
#[inline]
pub fn rot_z_rad(z_rad: f32) -> Mat4 {
    Mat4::from_rotation_z(z_rad)
}

/// Pitch only — discard bowl hover tilt, simple camera-facing leans.
#[inline]
pub fn rot_x_rad(x_rad: f32) -> Mat4 {
    Mat4::from_rotation_x(x_rad)
}

/// `T(pivot) * Rx(angle) * T(-pivot)` — hand tile tilt about its bottom-front edge.
#[inline]
pub fn rotation_around_point_x_rad(pivot: Vec3, angle_rad: f32) -> Mat4 {
    Mat4::from_translation(pivot) * Mat4::from_rotation_x(angle_rad) * Mat4::from_translation(-pivot)
}

/// Table procedural mesh is already in **XY** with normal **+Z**; no rotation.
#[inline]
pub fn table_mesh_lay_flat() -> Mat4 {
    Mat4::IDENTITY
}

/// Mahjong tile mesh from [`crate::render::tile_glb`] after `normalize_mesh`:
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

/// Extruded score popups: yaw around **Z** then pitch around **X** (pitch includes the `-π/2` camera-face term in call sites).
#[inline]
pub fn score_popup_glyph_rot_rad(rotation_z: f32, rotation_x: f32) -> Mat4 {
    Mat4::from_rotation_z(rotation_z) * Mat4::from_rotation_x(rotation_x)
}

/// Same as `Mat4::from_euler(glam::EulerRot::XYZ, …)` — hand tiles + showcase tiles.
#[inline]
pub fn rot_euler_xyz_rad(rx: f32, ry: f32, rz: f32) -> Mat4 {
    Mat4::from_euler(glam::EulerRot::XYZ, rx, ry, rz)
}

/// `Translation * rotation * scale` — standard lit-mesh instance pose on the table.
/// Use [`Mat4::IDENTITY`] for rotation when the mesh has no orientation (axis-aligned boxes).
/// For ribbon anchor poses with no per-axis scale yet, pass **`Vec3::splat(1.0)`**
/// so the result is **`T * R`** only.
#[inline]
pub fn translate_rot_scale(center: Vec3, rotation: Mat4, scale: Vec3) -> Mat4 {
    Mat4::from_translation(center) * rotation * Mat4::from_scale(scale)
}

/// Zodiac ribbon submesh after the anchor [`translate_rot_scale`]: offset along **local -Y**
/// (mesh hangs downward), then non-uniform scale. Use `local_offset_y = 0` for the top cap.
#[inline]
pub fn ribbon_submesh(parent: Mat4, local_offset_y: f32, scale: Vec3) -> Mat4 {
    parent * Mat4::from_translation(Vec3::new(0.0, local_offset_y, 0.0)) * Mat4::from_scale(scale)
}
