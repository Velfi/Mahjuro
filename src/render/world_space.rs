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
//!
//! Asset-local frames map into this space via [`crate::render::table_transform`] — e.g.
//! [`crate::render::table_transform::tile_mesh_local_to_world`],
//! [`crate::render::table_transform::mesh_y_thickness_along_local_y_to_z_up`]. Lit meshes use
//! [`crate::render::table_transform::translate_rot_scale`] with world-space centers.
//!
//! [`DrawCmd`](crate::render::draw_cmd) packs `(px, py, lift)` into [`WorldSurfaceAnchor`](crate::render::draw_cmd::WorldSurfaceAnchor);
//! the third value is **+Z** lift above the felt.

use glam::Vec3;

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

/// A position expressed as **normalized screen fractions** plus a physical lift height.
///
/// - `nx` — horizontal fraction: `0.0` = left edge, `1.0` = right edge.
/// - `ny` — vertical fraction:   `0.0` = top edge,  `1.0` = bottom edge.
/// - `lift_mm` — height above the felt in **millimeters** (not screen-relative).
///
/// Convert to the `[pixel_x, pixel_y, lift_z]` triple expected by placement
/// structs with [`ScreenPos::to_pixel_triple`].
///
/// # Screen-size invariance
///
/// Because `nx` / `ny` are fractions of the current window dimensions, an object
/// placed at `nx = 0.5` is always centered horizontally regardless of resolution.
/// Only `lift_mm` is in physical units — a 36 mm prop stays 36 mm tall as the
/// window scales.
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
#[derive(Clone, Copy, Debug)]
pub struct PlacementAnchor {
    pub anchor: LayoutAnchorPx,
    pub rot_y: f32,
    pub scale: f32,
}
