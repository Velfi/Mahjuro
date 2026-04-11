//! Layout pixels → 3D table world space.
//!
//! The gameplay camera looks at a horizontal table in the **XZ** plane (`y = 0` is the felt).
//! Screen pixel coordinates map to positions on that plane; the **second** world axis (**Y**)
//! is height above the wood.
//!
//! Mapping (same as [`crate::render::wgpu_renderer`]’s historical `pixel_to_world` closure):
//!
//! - `world_x = pixel_x - window_w * 0.5`
//! - `world_z = pixel_y - window_h * 0.5` (pixel **y** grows downward → larger **world z** is
//!   toward the near edge of the table / the player)
//! - `world_y = lift_y` — vertical offset above the table surface
//!
//! [`DrawCmd`](crate::render::draw_cmd) placements pack `(px, py, lift_y)` into
//! [`TableSurfaceAnchor`](crate::render::draw_cmd::TableSurfaceAnchor); that third component is
//! **not** world Z.

use glam::Vec3;

/// Maps a layout pixel anchor and height above the table into world space.
#[inline]
pub fn pixel_to_table_world(
    window_w: f32,
    window_h: f32,
    px: f32,
    py: f32,
    lift_y: f32,
) -> Vec3 {
    Vec3::new(
        px - window_w * 0.5,
        lift_y,
        py - window_h * 0.5,
    )
}

/// Screen-space anchor for a mesh sitting on or above the table, before packing into
/// [`TableSurfaceAnchor`](crate::render::draw_cmd::TableSurfaceAnchor).
#[derive(Clone, Copy, Debug)]
pub struct TableAnchorPx {
    pub px: f32,
    pub py: f32,
    /// Height above the table plane (maps to world **+Y**).
    pub lift_y: f32,
}

impl TableAnchorPx {
    /// Packed `[px, py, lift_y]` for [`crate::render::draw_cmd`] placement structs.
    #[inline]
    pub fn to_draw_cmd_triple(self) -> [f32; 3] {
        [self.px, self.py, self.lift_y]
    }
}
