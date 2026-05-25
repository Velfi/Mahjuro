//! Screen-space layout helpers for [`crate::render::draw_cmd::ShowcaseTilePlacement`] rows.
//! Matches model construction in [`crate::render::wgpu_renderer::runtime::showcase_tiles`].

use glam::Vec3;

use crate::persistence::TilePreset;
use crate::render::draw_cmd::CameraParams;
use crate::render::table_transform::{
    rot_euler_xyz_rad, tile_mesh_local_to_world, translate_rot_scale,
};
use crate::render::wgpu_renderer::{LOCAL_X_EXTENT, LOCAL_Y_EXTENT, LOCAL_Z_EXTENT};
use crate::render::world_space::layout_anchor_to_world;

/// Max fraction of window width occupied by the pack-reveal row (projected silhouettes + gaps).
const PACK_REVEAL_ROW_MAX_W_FRAC: f32 = 0.72;
/// Upper cap on [`crate::render::draw_cmd::ShowcaseTilePlacement::size_px`] vs window height.
const PACK_REVEAL_TILE_SIZE_MAX_H_FRAC: f32 = 0.20;
/// Clearance between adjacent tiles as a fraction of each tile's projected silhouette width.
const PACK_REVEAL_SILHOUETTE_GAP_FRAC: f32 = 0.14;

#[derive(Clone, Copy, Debug)]
pub struct PackRevealRowLayout {
    pub tile_size: f32,
    /// Projected horizontal span of one tile at `tile_size` (scale 1), in layout pixels.
    pub silhouette_w: f32,
    pub gap_px: f32,
    /// Left edge of the first tile's projected silhouette (layout X).
    pub row_x0: f32,
    /// Distance between consecutive tile centers along X.
    pub step_px: f32,
}

/// Inputs for [`showcase_tile_projected_width_px`].
pub struct ShowcaseTileProjectParams<'a> {
    pub win_w: f32,
    pub win_h: f32,
    pub cam: &'a CameraParams,
    pub preset: TilePreset,
    pub center_px: [f32; 3],
    pub rotation_xyz_rad: [f32; 3],
    pub placement_scale: f32,
    pub size_px: f32,
}

/// Inputs for [`compute_pack_reveal_row_layout`].
pub struct PackRevealRowLayoutParams<'a> {
    pub win_w: f32,
    pub win_h: f32,
    pub cam: &'a CameraParams,
    pub preset: TilePreset,
    pub n: usize,
    pub row_lift: f32,
    pub nx: f32,
    pub ny: f32,
    pub rotation_xyz_rad: [f32; 3],
}

/// Axis-aligned screen bounds of a showcase tile mesh (layout pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShowcaseTileScreenBounds {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl ShowcaseTileScreenBounds {
    pub fn width(self) -> f32 {
        (self.max_x - self.min_x).max(1.0)
    }

    pub fn height(self) -> f32 {
        (self.max_y - self.min_y).max(1.0)
    }

    pub fn bottom(self) -> f32 {
        self.max_y
    }

    pub fn merge(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }
}

/// Gaps between a tile group's projected bottom edge and its underline / label row.
#[derive(Clone, Copy, Debug)]
pub struct ShowcaseTileLabelGaps {
    pub underline_gap: f32,
    pub underline_h: f32,
    pub label_text_gap: f32,
}

/// Screen-space label row anchored to merged projected tile bounds.
#[derive(Clone, Copy, Debug)]
pub struct ShowcaseTileGroupLabelAnchor {
    pub bounds: ShowcaseTileScreenBounds,
    pub underline_y: f32,
    pub label_y: f32,
}

/// Merged projected AABB for a row of showcase tiles sharing size / rotation / preset.
pub fn showcase_tile_merge_projected_group(
    cam: &crate::render::draw_cmd::CameraParams,
    win_w: f32,
    win_h: f32,
    preset: TilePreset,
    rotation_xyz_rad: [f32; 3],
    placement_scale: f32,
    size_px: f32,
    lift_z: f32,
    centers_xy: &[[f32; 2]],
) -> ShowcaseTileScreenBounds {
    let mut merged = ShowcaseTileScreenBounds {
        min_x: f32::INFINITY,
        min_y: f32::INFINITY,
        max_x: f32::NEG_INFINITY,
        max_y: f32::NEG_INFINITY,
    };
    for &[px, py] in centers_xy {
        let bounds = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
            win_w,
            win_h,
            cam,
            preset,
            center_px: [px, py, lift_z],
            rotation_xyz_rad,
            placement_scale,
            size_px,
        });
        merged = merged.merge(bounds);
    }
    merged
}

/// Underline and label baseline Y from merged projected bounds (guide / tutorial / lab).
pub fn showcase_tile_group_label_anchor(
    bounds: ShowcaseTileScreenBounds,
    gaps: ShowcaseTileLabelGaps,
) -> ShowcaseTileGroupLabelAnchor {
    let underline_y = bounds.bottom() + gaps.underline_gap;
    let label_y = underline_y + gaps.underline_h + gaps.label_text_gap;
    ShowcaseTileGroupLabelAnchor {
        bounds,
        underline_y,
        label_y,
    }
}

/// Projected screen-space AABB for one showcase tile.
///
/// Uses the same ray→`plane_z` center mapping, `0.85` short-edge factor, preset ratios,
/// and `tile_mesh_local_to_world` × Euler basis as the GPU showcase path.
pub fn showcase_tile_projected_bounds_px(
    p: &ShowcaseTileProjectParams<'_>,
) -> ShowcaseTileScreenBounds {
    let center = layout_anchor_to_world(
        p.win_w,
        p.win_h,
        Some(p.cam),
        p.center_px[0],
        p.center_px[1],
        p.center_px[2],
        true,
    );

    let tile_short_px = p.size_px * 0.85 * p.placement_scale;
    let tile_long_px = tile_short_px * p.preset.face_long_ratio();
    let tile_thickness_px = tile_short_px * p.preset.thickness_ratio();
    let scale = Vec3::new(
        tile_long_px / LOCAL_X_EXTENT,
        tile_thickness_px / LOCAL_Y_EXTENT,
        tile_short_px / LOCAL_Z_EXTENT,
    );

    let base_rotation = rot_euler_xyz_rad(
        p.rotation_xyz_rad[0],
        p.rotation_xyz_rad[1],
        p.rotation_xyz_rad[2],
    );
    let oriented = base_rotation * tile_mesh_local_to_world();
    let model = translate_rot_scale(center, oriented, scale);

    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for &corner in &[
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(-0.5, 0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
    ] {
        let world_c = model.transform_point3(corner);
        let (sx, sy) = p.cam.project_world_to_screen(p.win_w, p.win_h, world_c);
        min_x = min_x.min(sx);
        min_y = min_y.min(sy);
        max_x = max_x.max(sx);
        max_y = max_y.max(sy);
    }
    ShowcaseTileScreenBounds {
        min_x,
        min_y,
        max_x,
        max_y,
    }
}

/// Horizontal span in layout pixels of a showcase tile's axis-aligned screen bounds.
pub fn showcase_tile_projected_width_px(p: &ShowcaseTileProjectParams<'_>) -> f32 {
    showcase_tile_projected_bounds_px(p).width()
}

/// Row geometry for pack opening: largest `size_px` such that projected silhouettes fit in
/// [`PACK_REVEAL_ROW_MAX_W_FRAC`] of the window with [`PACK_REVEAL_SILHOUETTE_GAP_FRAC`] spacing.
pub fn compute_pack_reveal_row_layout(p: &PackRevealRowLayoutParams<'_>) -> PackRevealRowLayout {
    let h = p.win_h.max(1.0);
    let w = p.win_w.max(1.0);
    let n_tiles = p.n.max(1) as f32;

    let cy = h * 0.5 + h * p.ny;
    let cx_ref = w * 0.5 + w * p.nx;
    let center_ref = [cx_ref, cy, p.row_lift];

    let gap_frac = PACK_REVEAL_SILHOUETTE_GAP_FRAC;
    let denom = n_tiles + (n_tiles - 1.0).max(0.0) * gap_frac;
    let target_sil_w = (w * PACK_REVEAL_ROW_MAX_W_FRAC) / denom.max(1e-6);

    let max_cap = h * PACK_REVEAL_TILE_SIZE_MAX_H_FRAC;

    let width_at = |size: f32| {
        showcase_tile_projected_width_px(&ShowcaseTileProjectParams {
            win_w: w,
            win_h: h,
            cam: p.cam,
            preset: p.preset,
            center_px: center_ref,
            rotation_xyz_rad: p.rotation_xyz_rad,
            placement_scale: 1.0,
            size_px: size,
        })
    };

    let mut tile_size = max_cap;
    if width_at(max_cap) > target_sil_w {
        let mut lo = 0.0_f32;
        let mut hi = max_cap;
        for _ in 0..28 {
            let mid = (lo + hi) * 0.5;
            if width_at(mid) <= target_sil_w {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        tile_size = lo;
    }

    let silhouette_w = width_at(tile_size);
    let gap_px = silhouette_w * gap_frac;
    let step_px = silhouette_w + gap_px;
    let total_w = n_tiles * silhouette_w + (n_tiles - 1.0).max(0.0) * gap_px;
    let row_x0 = (w - total_w) * 0.5 + w * p.nx;

    PackRevealRowLayout {
        tile_size,
        silhouette_w,
        gap_px,
        row_x0,
        step_px,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::TilePreset;
    use crate::scenes::shop::shop_celebration_camera;

    #[test]
    fn pack_reveal_row_tiles_are_readable_on_celebration_camera() {
        let w = 1920.0_f32;
        let h = 1080.0_f32;
        let cam = shop_celebration_camera(w, h, crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE);
        let rotation = [32.0_f32.to_radians(), 0.0, std::f32::consts::PI];
        let row = compute_pack_reveal_row_layout(&PackRevealRowLayoutParams {
            win_w: w,
            win_h: h,
            cam: &cam,
            preset: TilePreset::Chinese,
            n: 4,
            row_lift: 0.0,
            nx: 0.0,
            ny: 0.14,
            rotation_xyz_rad: rotation,
        });
        assert!(
            row.tile_size >= h * 0.10,
            "tile_size {} should be a readable fraction of window height",
            row.tile_size
        );
        assert!(
            row.silhouette_w >= h * 0.12,
            "projected silhouette_w {} too narrow for pack reveal",
            row.silhouette_w
        );
    }
}
