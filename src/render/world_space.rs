//! **World space** — single right-handed **Z-up** frame using standard 3D conventions:
//! **+X** right, **+Y** forward (away from player, into the table), **+Z** up from the felt.
//! The table is the plane **`z = 0`** (horizontal **XY**).
//!
//! ## Camera geometry
//!
//! The gameplay camera sits at large **−Y** (behind / in front of the player), elevated in **+Z**,
//! looking toward **+Y** (into the table). With `up = +Z` and `look_at_rh`:
//!
//! ```text
//! forward = normalize(target − eye)  →  f.y > 0
//! right   = forward × up = f × (0,0,1)  →  r.x = f.y > 0  →  camera right = +X  ✓
//! ```
//!
//! ## Pixel-to-world mapping
//!
//! ```text
//! world_x =  pixel_x − window_w / 2   (screen-left → −X, screen-right → +X)
//! world_y =  window_h / 2 − pixel_y   (screen-top → +Y far, screen-bottom → −Y near player)
//! world_z =  lift                      (+Z above the felt)
//! ```
//!
//! **Placement code uses plain pixel-space values** (`w * 0.80` = right side of screen).
//! [`pixel_to_world`] handles the sign conventions transparently.
//! Perspective scenes with [`CameraParams`] use [`layout_anchor_to_world`] instead.
//!
//! Asset-local frames map into this space via [`crate::render::table_transform`] — e.g.
//! [`crate::render::table_transform::tile_mesh_local_to_world`],
//! [`crate::render::table_transform::mesh_y_thickness_along_local_y_to_z_up`]. Lit meshes use
//! [`crate::render::table_transform::translate_rot_scale`] with world-space centers.
//!
//! [`DrawCmd`](crate::render::draw_cmd) packs `(px, py, lift)` into [`WorldSurfaceAnchor`](crate::render::draw_cmd::WorldSurfaceAnchor);
//! the third value is **+Z** lift above the felt.
//!
//! **Saved props** use [`crate::ui::placement::Placement`] — see
//! `docs/agents/scene-placement.md`. This module’s [`PlacementAnchor`] is only
//! for lightweight HUD-style anchors (reel, glyphs), not `ui::placement::PlacementAnchor`.

use glam::{Mat4, Vec3, Vec4};

use crate::render::draw_cmd::CameraParams;

/// Maps layout pixel coordinates plus vertical lift into world space (Z-up, standard conventions).
///
/// - **X** — `pixel_x − window_w / 2`. Screen-left (px=0) → world **−X**; screen-right →
///   world **+X**.
/// - **Y** — `window_h / 2 − pixel_y`. Screen-top (py=0) → world **+Y** (far side of table);
///   screen-bottom (py=h) → world **−Y** (near player).
/// - **Z** — `lift` above the felt (**+Z** up).
#[inline]
pub fn pixel_to_world(window_w: f32, window_h: f32, px: f32, py: f32, lift: f32) -> Vec3 {
    Vec3::new(px - window_w * 0.5, window_h * 0.5 - py, lift)
}

/// Maps a layout-space `(px, py, lift)` anchor to world space.
///
/// When `use_ray_plane` and [`CameraParams`] are set, uses [`world_on_camera_ray_plane_z`]
/// (guide / tutorial / shop showcase). Gameplay uses [`pixel_to_world`] with `use_ray_plane = false`
/// even when `camera_override` drives the view matrix.
#[inline]
pub fn layout_anchor_to_world(
    window_w: f32,
    window_h: f32,
    cam: Option<&CameraParams>,
    px: f32,
    py: f32,
    lift: f32,
    use_ray_plane: bool,
) -> Vec3 {
    if use_ray_plane && let Some(cam) = cam {
        world_on_camera_ray_plane_z(window_w, window_h, cam, px, py, lift)
    } else {
        pixel_to_world(window_w, window_h, px, py, lift)
    }
}

/// Inverse of [`pixel_to_world`] — packs world XYZ into [`crate::render::draw_cmd::Object3d::pos`] anchor form.
#[inline]
pub fn surface_anchor_from_world_xyz(window_w: f32, window_h: f32, world: Vec3) -> [f32; 3] {
    [world.x + window_w * 0.5, window_h * 0.5 - world.y, world.z]
}

/// World-space point where the camera ray through layout pixel `(px, py)` meets the
/// horizontal plane `world_z = plane_z`.
///
/// [`pixel_to_world`] is **not** the inverse of perspective projection; props placed at
/// new screen locations while reusing an existing shop-style camera would otherwise drift
/// on screen. This helper matches the view-projection construction in
/// [`crate::render::wgpu_renderer::runtime::camera::CameraFrame::build`].
pub fn world_on_camera_ray_plane_z(
    window_w: f32,
    window_h: f32,
    cam: &CameraParams,
    px: f32,
    py: f32,
    plane_z: f32,
) -> Vec3 {
    let w = window_w.max(1e-6);
    let h = window_h.max(1e-6);
    let aspect = (w / h).max(1e-6);
    let eye = Vec3::from_array(cam.eye);
    let target = Vec3::from_array(cam.target);
    let up = Vec3::from_array(cam.up);
    let fov_y = cam.fovy_deg.to_radians();
    let view_mat = Mat4::look_at_rh(eye, target, up);
    let (near, far) = cam.clip_planes(h);
    let proj = Mat4::perspective_rh(fov_y, aspect, near, far);
    let view_proj = proj * view_mat;
    let inv_vp = view_proj.inverse();

    let ndc_x = (px / w) * 2.0 - 1.0;
    let ndc_y = 1.0 - (py / h) * 2.0;
    let near_clip = Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
    let far_clip = Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
    let nw = inv_vp * near_clip;
    let fw = inv_vp * far_clip;
    if nw.w.abs() < 1e-8 || fw.w.abs() < 1e-8 {
        return pixel_to_world(window_w, window_h, px, py, plane_z);
    }
    let near_w = nw.truncate() / nw.w;
    let far_w = fw.truncate() / fw.w;
    let dir = far_w - near_w;
    let dz = dir.z;
    if dz.abs() < 1e-8 {
        return pixel_to_world(window_w, window_h, px, py, plane_z);
    }
    let t = (plane_z - near_w.z) / dz;
    near_w + dir * t
}

/// Encode a world-space mesh center into `[px, py, lift]` for [`crate::render::draw_cmd::Object3d::pos`]
/// so [`pixel_to_world`] returns `center` exactly.
#[inline]
pub fn object3d_pos_triple_for_world_center(
    window_w: f32,
    window_h: f32,
    center: Vec3,
) -> [f32; 3] {
    [
        center.x + window_w * 0.5,
        window_h * 0.5 - center.y,
        center.z,
    ]
}

/// Convenience: ray-plane world hit → `Object3d.pos` triple (perspective-correct anchor).
///
/// The GPU still decodes via [`pixel_to_world`] unless [`ShowcaseRenderHints::object3d_uses_ray_plane`]
/// is set — shop/archive store this triple for that round-trip.
#[inline]
pub fn object3d_pos_for_screen_at_world_z(
    window_w: f32,
    window_h: f32,
    cam: &CameraParams,
    px: f32,
    py: f32,
    plane_z: f32,
) -> [f32; 3] {
    let c = world_on_camera_ray_plane_z(window_w, window_h, cam, px, py, plane_z);
    object3d_pos_triple_for_world_center(window_w, window_h, c)
}

/// Top edge **`py`** (top-down layout pixels, `py` increases downward) for a rectangle
/// with height `rect_h_px` and an empty band `margin_bottom_px` below it to the window bottom.
///
/// Not every caller needs this helper (some scenes pin both corners via layout anchors).
#[allow(dead_code)]
#[inline]
pub fn layout_py_top_from_bottom_margin(
    window_h: f32,
    rect_h_px: f32,
    margin_bottom_px: f32,
) -> f32 {
    window_h - margin_bottom_px - rect_h_px
}

/// Normalized horizontal / vertical fractions → layout **`px`**, **`py`** (top-down).
///
/// - **`nx`** — `0` = left edge, `1` = right edge (maps to `px = nx * window_w`).
/// - **`ny`** — `0` = top edge, `1` = bottom edge (maps to `py = ny * window_h`).
///
/// Use this for responsive layout that still ends in the `(px, py)` form consumed by
/// [`pixel_to_world`] and lit-mesh placement.
#[inline]
pub fn layout_px_py_from_norm(window_w: f32, window_h: f32, nx: f32, ny: f32) -> (f32, f32) {
    (nx * window_w, ny * window_h)
}

/// Layout-space anchor (pixels + lift) before packing into [`WorldSurfaceAnchor`](crate::render::draw_cmd::WorldSurfaceAnchor).
#[derive(Clone, Copy, Debug)]
pub struct LayoutAnchorPx {
    pub px: f32,
    pub py: f32,
    /// Height above the felt in world **+Z**.
    pub lift_z: f32,
}

impl LayoutAnchorPx {
    /// Packed `[px, py, lift]` for [`crate::render::draw_cmd`] placement structs.
    #[inline]
    pub fn to_draw_cmd_triple(self) -> [f32; 3] {
        [self.px, self.py, self.lift_z]
    }
}

/// Full placement for 3D helpers that position, orient, and size a prop:
/// the layout-space anchor plus yaw (camera-facing rotation inherited from
/// the parent face) and a uniform scale multiplier. Shared by score reel,
/// tooltip panels, and cascade HUD placements so each one doesn't reinvent
/// its own 5-field param list.
#[allow(dead_code)] // Built by `ScoreReel::placements` when the 3D reel draw path is enabled.
#[derive(Clone, Copy, Debug)]
pub struct PlacementAnchor {
    pub anchor: LayoutAnchorPx,
    pub rot_y: f32,
    pub scale: f32,
}

#[cfg(test)]
mod layout_helpers_tests {
    use super::*;

    #[test]
    fn layout_py_top_from_bottom_margin_matches_explicit() {
        let h = 800.0;
        let rect_h = 600.0;
        let m = 128.0;
        let got = layout_py_top_from_bottom_margin(h, rect_h, m);
        assert!((got - (h - m - rect_h)).abs() < 1e-5);
    }

    #[test]
    fn layout_px_py_from_norm_scales_window() {
        let (px, py) = layout_px_py_from_norm(100.0, 200.0, 0.5, 0.25);
        assert!((px - 50.0).abs() < 1e-5);
        assert!((py - 50.0).abs() < 1e-5);
    }
}
