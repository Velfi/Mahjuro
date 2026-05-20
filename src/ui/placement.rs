//! Unified scene-placement primitives.
//!
//! See [`docs/agents/scene-placement.md`](../../../docs/agents/scene-placement.md).

/// A manually-placeable object's position and rotation.
///
/// `nx` / `ny` are normalized window fractions (0–1, may go outside for
/// off-screen placements). `lift_mm` is physical millimeters above the felt.
/// Rotation degrees are applied in the renderer as **`Rz * Ry * Rx`** on the
/// mesh model basis (see `apply_placement_rotation` in `wgpu_renderer.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    /// 0 = left edge, 1 = right edge.
    pub nx: f32,
    /// 0 = top edge, 1 = bottom edge.
    pub ny: f32,
    /// Physical height above the felt, in millimeters.
    pub lift_mm: f32,
    /// Rotation around world X (pitch), degrees.
    pub rx_deg: f32,
    /// Rotation around world Y (yaw), degrees.
    pub ry_deg: f32,
    /// Rotation around world Z (roll), degrees.
    pub rz_deg: f32,
}

impl Placement {
    /// Construct a placement at the given screen fraction and lift, with zero rotation.
    pub const fn at(nx: f32, ny: f32, lift_mm: f32) -> Self {
        Self {
            nx,
            ny,
            lift_mm,
            rx_deg: 0.0,
            ry_deg: 0.0,
            rz_deg: 0.0,
        }
    }

    pub fn is_finite(&self) -> bool {
        self.nx.is_finite()
            && self.ny.is_finite()
            && self.lift_mm.is_finite()
            && self.rx_deg.is_finite()
            && self.ry_deg.is_finite()
            && self.rz_deg.is_finite()
    }

    #[allow(dead_code)]
    #[inline]
    pub fn rotation_xyz_rad(&self) -> [f32; 3] {
        crate::render::table_transform::euler_xyz_rad_from_deg(
            self.rx_deg,
            self.ry_deg,
            self.rz_deg,
        )
    }
}

/// Anchor derived from a [`Placement`] for constructing an `Object3d`.
///
/// Folds `nx`/`ny`/`lift_mm` into the draw-site anchor and leaves mesh rotation
/// at `base_rotation`. The renderer applies `rx_deg`/`ry_deg`/`rz_deg` via
/// `placement_rotations` when `arrange_name` is set — do not bake those degrees
/// into `Object3d.rotation` at the construction site.
#[derive(Clone, Copy, Debug)]
pub struct PlacementAnchor {
    pub pos: [f32; 3],
    pub rotation: glam::Mat4,
    pub arrange_name: &'static str,
}

impl PlacementAnchor {
    #[inline]
    pub fn object3d_rotation(&self) -> [f32; 3] {
        crate::render::table_transform::mat4_to_euler_xyz_rad(self.rotation)
    }

    pub fn new(
        base_pos: [f32; 3],
        base_rotation: glam::Mat4,
        placement: &Placement,
        arrange_name: &'static str,
        layout: &crate::ui::layout::LayoutResult,
    ) -> Self {
        Self {
            pos: [
                base_pos[0] + placement.nx * layout.window_w,
                base_pos[1] + placement.ny * layout.window_h,
                base_pos[2] + layout.mm(placement.lift_mm),
            ],
            rotation: base_rotation,
            arrange_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn placement_rotation_xyz_rad_matches_degrees() {
        let p = Placement {
            rx_deg: 90.0,
            ry_deg: -45.0,
            rz_deg: 0.0,
            ..Placement::default()
        };
        let r = p.rotation_xyz_rad();
        assert!(approx(r[0], std::f32::consts::FRAC_PI_2));
        assert!(approx(r[1], -std::f32::consts::FRAC_PI_4));
        assert!(approx(r[2], 0.0));
    }

    #[test]
    fn placement_is_finite_rejects_nan() {
        let mut p = Placement::at(0.5, 0.5, 10.0);
        assert!(p.is_finite());
        p.nx = f32::NAN;
        assert!(!p.is_finite());
    }
}
