//! Screen-space layout helpers for [`crate::render::draw_cmd::ShowcaseTilePlacement`] rows.
//! Matches model construction in [`crate::render::wgpu_renderer::runtime::showcase_tiles`].

use glam::Vec3;

use crate::persistence::TilePreset;
use crate::render::draw_cmd::CameraParams;
use crate::render::table_transform::{rot_euler_xyz_rad, tile_mesh_local_to_world};
use crate::render::world_space::world_on_camera_ray_plane_z;

/// Max fraction of window width occupied by the pack-reveal row (projected silhouettes + gaps).
const PACK_REVEAL_ROW_MAX_W_FRAC: f32 = 0.48;
/// Upper cap on [`crate::render::draw_cmd::ShowcaseTilePlacement::size_px`] vs window height.
const PACK_REVEAL_TILE_SIZE_MAX_H_FRAC: f32 = 0.048;
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

/// Horizontal span in layout pixels of a showcase tile's axis-aligned screen bounds.
///
/// Uses the same ray→`plane_z` center mapping, `0.85` short-edge factor, preset ratios,
/// and `tile_mesh_local_to_world` × Euler basis as the GPU showcase path.
pub fn showcase_tile_projected_width_px(
    w: f32,
    h: f32,
    cam: &CameraParams,
    preset: TilePreset,
    center_px: [f32; 3],
    rotation_xyz_rad: [f32; 3],
    placement_scale: f32,
    size_px: f32,
) -> f32 {
    let center = world_on_camera_ray_plane_z(w, h, cam, center_px[0], center_px[1], center_px[2]);

    let tile_short_px = size_px * 0.85 * placement_scale;
    let tile_long_px = tile_short_px * preset.face_long_ratio();
    let tile_thickness_px = tile_short_px * preset.thickness_ratio();

    let lx = tile_long_px * 0.5;
    let ly = tile_thickness_px * 0.5;
    let lz = tile_short_px * 0.5;

    let base_rotation = rot_euler_xyz_rad(rotation_xyz_rad[0], rotation_xyz_rad[1], rotation_xyz_rad[2]);
    let oriented = base_rotation * tile_mesh_local_to_world();

    let corners = [
        Vec3::new(-lx, -ly, -lz),
        Vec3::new(lx, -ly, -lz),
        Vec3::new(-lx, ly, -lz),
        Vec3::new(lx, ly, -lz),
        Vec3::new(-lx, -ly, lz),
        Vec3::new(lx, -ly, lz),
        Vec3::new(-lx, ly, lz),
        Vec3::new(lx, ly, lz),
    ];

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    for c in corners {
        let world_c = center + oriented.transform_point3(c);
        let (sx, _) = cam.project_world_to_screen(w, h, world_c);
        min_x = min_x.min(sx);
        max_x = max_x.max(sx);
    }
    (max_x - min_x).max(1.0)
}

/// Row geometry for pack opening: largest `size_px` such that projected silhouettes fit in
/// [`PACK_REVEAL_ROW_MAX_W_FRAC`] of the window with [`PACK_REVEAL_SILHOUETTE_GAP_FRAC`] spacing.
pub fn compute_pack_reveal_row_layout(
    w: f32,
    h: f32,
    cam: &CameraParams,
    preset: TilePreset,
    n: usize,
    row_lift: f32,
    nx: f32,
    ny: f32,
    rotation_xyz_rad: [f32; 3],
) -> PackRevealRowLayout {
    let h = h.max(1.0);
    let w = w.max(1.0);
    let n_tiles = n.max(1) as f32;

    let cy = h * ny;
    let cx_ref = w * 0.5 + w * nx;
    let center_ref = [cx_ref, cy, row_lift];

    let gap_frac = PACK_REVEAL_SILHOUETTE_GAP_FRAC;
    let denom = n_tiles + (n_tiles - 1.0).max(0.0) * gap_frac;
    let target_sil_w = (w * PACK_REVEAL_ROW_MAX_W_FRAC) / denom.max(1e-6);

    let max_cap = h * PACK_REVEAL_TILE_SIZE_MAX_H_FRAC;

    let width_at = |size: f32| {
        showcase_tile_projected_width_px(
            w,
            h,
            cam,
            preset,
            center_ref,
            rotation_xyz_rad,
            1.0,
            size,
        )
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
    let row_x0 = (w - total_w) * 0.5 + w * nx;

    PackRevealRowLayout {
        tile_size,
        silhouette_w,
        gap_px,
        row_x0,
        step_px,
    }
}
