//! Responsive anchors for 3D props that are laid out from **screen / layout pixels**, then drawn in
//! [`crate::render::world_space`].
//!
//! ## Pipeline
//!
//! 1. Describe horizontal / vertical placement with [`ScreenAxis`] (**normalized** `0..1` or **absolute
//!    pixels**) plus optional pixel nudges.
//! 2. [`ScreenAnchor::resolve`] → [`crate::render::world_space::LayoutAnchorPx`] →
//!    [`crate::render::world_space::LayoutAnchorPx::to_draw_cmd_triple`] fills [`crate::render::draw_cmd::Object3d::pos`].
//! 3. Build rotation with [`crate::render::table_transform`] helpers and/or
//!    [`crate::render::draw_cmd::camera_facing_euler_xyz_rad`] (store euler triples on [`crate::render::draw_cmd::Object3d`]);
//!    use [`crate::render::table_transform::mat4_to_euler_xyz_rad`] when composing extra `Mat4`s.
//! 4. Optional: [`ScreenSpacePose::model_matrix`] composes center + rotation + extents like the renderer's
//!    `translate_rot_scale` path (useful for debug overlays or CPU picking that mirror GPU placement).
//!
//! [`ScreenLift::FractionOfWindowHeight`] tracks how menus scale camera depth with `window_h` — use it when lift
//! should grow/shrink with resolution the same way tuned constants often do.
//!
//! Several constructors exist so different scenes can pick the clearest form; not every helper is used from the
//! binary yet — treat this module as a shared toolkit.

#![allow(dead_code)]

use glam::{Mat4, Vec2, Vec3};

use crate::render::draw_cmd::CameraParams;
use crate::render::table_transform::{
    mat4_to_euler_xyz_rad, rot_euler_xyz_rad, translate_rot_scale,
};
use crate::render::world_space::{LayoutAnchorPx, pixel_to_world};

/// One scalar placement axis: fraction of window width/height, or absolute layout pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenAxis {
    /// `0` = left or top edge, `1` = right or bottom (matches [`crate::render::world_space::layout_px_py_from_norm`]).
    Norm(f32),
    /// Layout pixels from the window origin (`x` from left, `y` from top, `py` down).
    Px(f32),
}

impl ScreenAxis {
    #[inline]
    fn resolve(self, span: f32) -> f32 {
        match self {
            ScreenAxis::Norm(t) => t * span,
            ScreenAxis::Px(p) => p,
        }
    }
}

/// A point in layout space before lift is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPoint {
    pub x: ScreenAxis,
    pub y: ScreenAxis,
}

impl ScreenPoint {
    #[inline]
    pub const fn from_norm(nx: f32, ny: f32) -> Self {
        Self {
            x: ScreenAxis::Norm(nx),
            y: ScreenAxis::Norm(ny),
        }
    }

    #[inline]
    pub const fn from_px(px: f32, py: f32) -> Self {
        Self {
            x: ScreenAxis::Px(px),
            y: ScreenAxis::Px(py),
        }
    }

    /// Resolve to layout `(px, py)` for the given window size.
    #[inline]
    pub fn resolve_xy(self, window_w: f32, window_h: f32) -> (f32, f32) {
        (self.x.resolve(window_w), self.y.resolve(window_h))
    }
}

/// Vertical lift for the anchor — same meaning as `Object3d.pos[2]` (world **+Z** above the felt).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenLift {
    /// Absolute world units (already matches renderer/convention — pass through).
    WorldZ(f32),
    /// `factor * window_h` — responsive lift tied to window height.
    FractionOfWindowHeight(f32),
}

impl ScreenLift {
    #[inline]
    pub fn resolve(self, window_h: f32) -> f32 {
        match self {
            ScreenLift::WorldZ(z) => z,
            ScreenLift::FractionOfWindowHeight(k) => k * window_h,
        }
    }
}

/// Fully specified screen anchor: 2D point + optional pixel offset + lift.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenAnchor {
    pub point: ScreenPoint,
    /// Added after resolving [`ScreenPoint`] (layout pixels).
    pub offset_px: Vec2,
    pub lift: ScreenLift,
}

impl ScreenAnchor {
    #[inline]
    pub fn new(point: ScreenPoint, offset_px: Vec2, lift: ScreenLift) -> Self {
        Self {
            point,
            offset_px,
            lift,
        }
    }

    /// Normalize-only anchor at `(nx, ny)` with world-Z lift.
    #[inline]
    pub fn norm(nx: f32, ny: f32, lift_z: f32) -> Self {
        Self {
            point: ScreenPoint::from_norm(nx, ny),
            offset_px: Vec2::ZERO,
            lift: ScreenLift::WorldZ(lift_z),
        }
    }

    #[inline]
    pub fn resolve(self, window_w: f32, window_h: f32) -> LayoutAnchorPx {
        let (px, py) = self.point.resolve_xy(window_w, window_h);
        LayoutAnchorPx {
            px: px + self.offset_px.x,
            py: py + self.offset_px.y,
            lift_z: self.lift.resolve(window_h),
        }
    }

    /// `[px, py, lift]` for [`crate::render::draw_cmd::Object3d::pos`].
    #[inline]
    pub fn object3d_pos(self, window_w: f32, window_h: f32) -> [f32; 3] {
        self.resolve(window_w, window_h).to_draw_cmd_triple()
    }

    /// World-space center after the same mapping the renderer uses for `Object3d`.
    #[inline]
    pub fn world_center(self, window_w: f32, window_h: f32) -> Vec3 {
        let a = self.resolve(window_w, window_h);
        pixel_to_world(window_w, window_h, a.px, a.py, a.lift_z)
    }
}

/// Resolved anchor plus orientation — enough to build a lit-mesh model matrix or fill [`crate::render::draw_cmd::Object3d`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenSpacePose {
    pub anchor: ScreenAnchor,
    /// Euler **XYZ** radians — same as [`crate::render::draw_cmd::Object3d::rotation`].
    pub rotation: [f32; 3],
}

impl ScreenSpacePose {
    #[inline]
    pub fn new(anchor: ScreenAnchor, rotation: [f32; 3]) -> Self {
        Self { anchor, rotation }
    }

    /// `extra * R(base)` — `extra` applied on the left (matches [`crate::render::draw_cmd::Object3d`] docs).
    #[inline]
    pub fn with_rotation_prepend(self, extra: Mat4) -> Self {
        let base = rot_euler_xyz_rad(self.rotation[0], self.rotation[1], self.rotation[2]);
        Self {
            anchor: self.anchor,
            rotation: mat4_to_euler_xyz_rad(extra * base),
        }
    }

    /// Pitch-only billboard facing `camera` ([`crate::render::draw_cmd::camera_facing_euler_xyz_rad`]).
    #[inline]
    pub fn facing_camera(anchor: ScreenAnchor, camera: &CameraParams) -> Self {
        Self {
            anchor,
            rotation: crate::render::draw_cmd::camera_facing_euler_xyz_rad(
                camera.eye,
                camera.target,
            ),
        }
    }

    #[inline]
    pub fn world_center(&self, window_w: f32, window_h: f32) -> Vec3 {
        self.anchor.world_center(window_w, window_h)
    }

    /// `T * R * S` at the resolved anchor (same composition as the object3d placement path).
    #[inline]
    pub fn model_matrix(&self, window_w: f32, window_h: f32, extents: Vec3) -> Mat4 {
        let c = self.world_center(window_w, window_h);
        let rot = rot_euler_xyz_rad(self.rotation[0], self.rotation[1], self.rotation[2]);
        translate_rot_scale(c, rot, extents)
    }

    #[inline]
    pub fn object3d_pos(&self, window_w: f32, window_h: f32) -> [f32; 3] {
        self.anchor.object3d_pos(window_w, window_h)
    }
}

/// Scales a tuned constant from a reference height (e.g. `2104`) to the current window — same pattern as menu cameras.
#[inline]
pub fn scale_for_window_height(value_at_ref: f32, window_h: f32, reference_h: f32) -> f32 {
    let rh = reference_h.max(1.0);
    value_at_ref * (window_h / rh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_norm_matches_layout_px_py() {
        let w = 800.0;
        let h = 600.0;
        let a = ScreenAnchor::norm(0.25, 0.5, 12.0).resolve(w, h);
        assert!((a.px - 200.0).abs() < 1e-5);
        assert!((a.py - 300.0).abs() < 1e-5);
        assert!((a.lift_z - 12.0).abs() < 1e-5);
    }

    #[test]
    fn anchor_offset_px_applies_after_norm() {
        let w = 100.0;
        let h = 100.0;
        let anc = ScreenAnchor {
            point: ScreenPoint::from_norm(0.5, 0.5),
            offset_px: Vec2::new(10.0, -5.0),
            lift: ScreenLift::WorldZ(0.0),
        };
        let r = anc.resolve(w, h);
        assert!((r.px - 60.0).abs() < 1e-5);
        assert!((r.py - 45.0).abs() < 1e-5);
    }

    #[test]
    fn lift_fraction_scales_with_height() {
        let l = ScreenLift::FractionOfWindowHeight(0.1).resolve(720.0);
        assert!((l - 72.0).abs() < 1e-5);
    }

    #[test]
    fn world_center_matches_pixel_to_world() {
        let w = 640.0;
        let h = 480.0;
        let anc = ScreenAnchor {
            point: ScreenPoint::from_px(100.0, 400.0),
            offset_px: Vec2::ZERO,
            lift: ScreenLift::WorldZ(8.0),
        };
        let wc = anc.world_center(w, h);
        let exp = pixel_to_world(w, h, 100.0, 400.0, 8.0);
        assert!((wc - exp).length() < 1e-5);
    }

    #[test]
    fn scale_for_window_height_scales_linearly() {
        let v = scale_for_window_height(2104.0, 1052.0, 2104.0);
        assert!((v - 1052.0).abs() < 1e-4);
    }

    #[test]
    fn pose_composes_like_translate_rot_scale() {
        let w = 800.0;
        let h = 600.0;
        let anchor = ScreenAnchor::new(
            ScreenPoint::from_norm(0.5, 0.5),
            Vec2::new(0.0, 0.0),
            ScreenLift::WorldZ(10.0),
        );
        let rot = mat4_to_euler_xyz_rad(
            Mat4::from_rotation_z(3.0_f32.to_radians())
                * Mat4::from_rotation_x(5.0_f32.to_radians())
                * Mat4::from_rotation_y((-12.0_f32).to_radians()),
        );
        let pose = ScreenSpacePose::new(anchor, rot).with_rotation_prepend(Mat4::IDENTITY);
        let ext = Vec3::new(40.0, 8.0, 60.0);
        let m = pose.model_matrix(w, h, ext);
        let wc = pose.world_center(w, h);
        assert!((m.w_axis.truncate() - wc).length() < 1e-4);
        assert_eq!(pose.object3d_pos(w, h), anchor.object3d_pos(w, h));

        let cam = CameraParams {
            eye: [0.0, -900.0, 1000.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 45.0,
        };
        let _ = ScreenSpacePose::facing_camera(anchor, &cam);
    }
}
