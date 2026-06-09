//! Shared orthographic cameras for doc-style tile rendering (guide, tutorial, wall ledger).
//!
//! Oblique [`doc_tile_camera`] matches the legacy guide perspective at reference height 1600.
//! Top-down [`wall_ledger_camera`] matches the wall ledger, chronicle strips, and yaku journal grid
//! (`pixel_to_world` placement).
//! [`CameraProjection::Orthographic`] keeps projected tile size uniform across each layout.

use crate::draw_cmd::{CameraParams, CameraProjection};

/// Euler rotation for oblique doc-tile showcase placements (guide / tutorial).
pub const DOC_TILE_ROTATION: [f32; 3] = [0.0, 0.0, std::f32::consts::PI];

/// Top-down grid tiles (wall ledger detail + grid cells).
pub const TOP_DOWN_TILE_ROTATION: [f32; 3] = DOC_TILE_ROTATION;

const DOC_REF_FOVY_DEG: f32 = 45.0;
/// Reference layout used to calibrate ortho `half_height` (matches guide at 2560×1600).
const DOC_REF_W: f32 = 2560.0;
const DOC_REF_H: f32 = 1600.0;
const DOC_CALIB_TILE_SIZE_PX: f32 = 120.0;

/// Canonical ortho camera for guide, tutorial, and tile-anchor lab doc presets.
pub fn doc_tile_camera(h: f32) -> CameraParams {
    let cam_scale = h / 1600.0;
    let eye = [0.0, -200.0 * cam_scale, 2040.0 * cam_scale];
    let target = [0.0, -50.0 * cam_scale, 0.0];
    let half_height = calibrated_ortho_half_height(
        h,
        eye,
        target,
        [0.0, 0.0, 1.0],
        true,
        DOC_TILE_ROTATION,
    );
    CameraParams {
        eye,
        target,
        up: [0.0, 0.0, 1.0],
        projection: CameraProjection::Orthographic { half_height },
        clip_near: None,
        clip_far: None,
    }
}

/// Top-down ortho camera for wall ledger grids (`pixel_to_world` placement).
pub fn wall_ledger_camera(h: f32) -> CameraParams {
    let cam_scale = h / 1600.0;
    let eye = [0.0, 0.0, 2040.0 * cam_scale];
    let target = [0.0, 0.0, 0.0];
    let half_height = calibrated_ortho_half_height(
        h,
        eye,
        target,
        [0.0, 1.0, 0.0],
        false,
        TOP_DOWN_TILE_ROTATION,
    );
    CameraParams {
        eye,
        target,
        up: [0.0, 1.0, 0.0],
        projection: CameraProjection::Orthographic { half_height },
        clip_near: None,
        clip_far: None,
    }
}

/// Match legacy perspective center-tile width at this window height.
fn calibrated_ortho_half_height(
    h: f32,
    eye: [f32; 3],
    target: [f32; 3],
    up: [f32; 3],
    use_ray_plane: bool,
    rotation: [f32; 3],
) -> f32 {
    use crate::showcase_tile_layout::{
        ShowcaseTileProjectParams, showcase_tile_projected_bounds_px,
    };

    let w = h * (DOC_REF_W / DOC_REF_H);
    let persp = CameraParams {
        eye,
        target,
        up,
        projection: CameraProjection::Perspective {
            fovy_deg: DOC_REF_FOVY_DEG,
        },
        clip_near: None,
        clip_far: None,
    };
    let target_w = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
        win_w: w,
        win_h: h,
        cam: &persp,
        preset: mahjuro_gfx_types::TilePreset::Chinese,
        center_px: [w * 0.5, h * 0.5, 0.0],
        rotation_xyz_rad: rotation,
        placement_scale: 1.0,
        size_px: DOC_CALIB_TILE_SIZE_PX,
        use_ray_plane,
    })
    .width();

    let base = CameraParams::ortho_half_height_from_perspective(DOC_REF_FOVY_DEG, eye, target);
    let mut lo = base * 0.78;
    let mut hi = base * 1.08;
    for _ in 0..28 {
        let mid = (lo + hi) * 0.5;
        let ortho = CameraParams {
            eye,
            target,
            up,
            projection: CameraProjection::Orthographic { half_height: mid },
            clip_near: None,
            clip_far: None,
        };
        let w_mid = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
            win_w: w,
            win_h: h,
            cam: &ortho,
            preset: mahjuro_gfx_types::TilePreset::Chinese,
            center_px: [w * 0.5, h * 0.5, 0.0],
            rotation_xyz_rad: rotation,
            placement_scale: 1.0,
            size_px: DOC_CALIB_TILE_SIZE_PX,
            use_ray_plane,
        })
        .width();
        if w_mid < target_w {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    (lo + hi) * 0.5
}

/// Legacy guide perspective camera (tile-anchor lab A/B only).
pub fn legacy_guide_perspective_camera(h: f32) -> CameraParams {
    let cam_scale = h / 1600.0;
    CameraParams {
        eye: [0.0, -200.0 * cam_scale, 2040.0 * cam_scale],
        target: [0.0, -50.0 * cam_scale, 0.0],
        up: [0.0, 0.0, 1.0],
        projection: CameraProjection::Perspective {
            fovy_deg: DOC_REF_FOVY_DEG,
        },
        clip_near: None,
        clip_far: None,
    }
}

/// Legacy wall-ledger top-down perspective (calibration reference).
pub fn legacy_wall_ledger_perspective_camera(h: f32) -> CameraParams {
    let cam_scale = h / 1600.0;
    CameraParams {
        eye: [0.0, 0.0, 2040.0 * cam_scale],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        projection: CameraProjection::Perspective {
            fovy_deg: DOC_REF_FOVY_DEG,
        },
        clip_near: None,
        clip_far: None,
    }
}

/// Legacy tutorial perspective camera (tile-anchor lab A/B only).
pub fn legacy_tutorial_perspective_camera(h: f32) -> CameraParams {
    let cam_scale = h / 1600.0;
    CameraParams {
        eye: [0.0, -220.0 * cam_scale, 1960.0 * cam_scale],
        target: [0.0, -40.0 * cam_scale, 0.0],
        up: [0.0, 0.0, 1.0],
        projection: CameraProjection::Perspective {
            fovy_deg: DOC_REF_FOVY_DEG,
        },
        clip_near: None,
        clip_far: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::showcase_tile_layout::{
        ShowcaseTileProjectParams, showcase_tile_projected_bounds_px,
    };
    use mahjuro_gfx_types::TilePreset;

    #[test]
    fn doc_ortho_matches_legacy_guide_center_tile_width() {
        let h = DOC_REF_H;
        let w = DOC_REF_W;
        let ortho = doc_tile_camera(h);
        let persp = legacy_guide_perspective_camera(h);
        let size_px = 120.0;
        let center = [w * 0.5, h * 0.5, 0.0];
        let rotation = DOC_TILE_ROTATION;
        let ortho_w = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
            win_w: w,
            win_h: h,
            cam: &ortho,
            preset: TilePreset::Chinese,
            center_px: center,
            rotation_xyz_rad: rotation,
            placement_scale: 1.0,
            size_px,
            use_ray_plane: true,
        })
        .width();
        let persp_w = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
            win_w: w,
            win_h: h,
            cam: &persp,
            preset: TilePreset::Chinese,
            center_px: center,
            rotation_xyz_rad: rotation,
            placement_scale: 1.0,
            size_px,
            use_ray_plane: true,
        })
        .width();
        let ratio = ortho_w / persp_w;
        assert!(
            (0.99..=1.01).contains(&ratio),
            "ortho center width {ortho_w} vs perspective {persp_w} (ratio {ratio})"
        );
    }

    #[test]
    fn doc_ortho_tile_width_invariant_across_screen_x() {
        let h = 1600.0;
        let w = 2560.0;
        let cam = doc_tile_camera(h);
        let size_px = 100.0;
        let y = h * 0.5;
        let rotation = DOC_TILE_ROTATION;
        let width_at = |px: f32| {
            showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
                win_w: w,
                win_h: h,
                cam: &cam,
                preset: TilePreset::Chinese,
                center_px: [px, y, 0.0],
                rotation_xyz_rad: rotation,
                placement_scale: 1.0,
                size_px,
                use_ray_plane: true,
            })
            .width()
        };
        let left = width_at(w * 0.12);
        let center = width_at(w * 0.5);
        let right = width_at(w * 0.88);
        assert!((left - center).abs() < 0.5, "left {left} center {center}");
        assert!((right - center).abs() < 0.5, "right {right} center {center}");
    }

    #[test]
    fn wall_ledger_ortho_matches_legacy_center_tile_width() {
        let h = DOC_REF_H;
        let w = DOC_REF_W;
        let ortho = wall_ledger_camera(h);
        let persp = legacy_wall_ledger_perspective_camera(h);
        let size_px = 80.0;
        let center = [w * 0.5, h * 0.5, 0.0];
        let rotation = TOP_DOWN_TILE_ROTATION;
        let ortho_w = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
            win_w: w,
            win_h: h,
            cam: &ortho,
            preset: TilePreset::Chinese,
            center_px: center,
            rotation_xyz_rad: rotation,
            placement_scale: 1.0,
            size_px,
            use_ray_plane: false,
        })
        .width();
        let persp_w = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
            win_w: w,
            win_h: h,
            cam: &persp,
            preset: TilePreset::Chinese,
            center_px: center,
            rotation_xyz_rad: rotation,
            placement_scale: 1.0,
            size_px,
            use_ray_plane: false,
        })
        .width();
        let ratio = ortho_w / persp_w;
        assert!(
            (0.98..=1.02).contains(&ratio),
            "wall ledger ortho {ortho_w} vs perspective {persp_w} (ratio {ratio})"
        );
    }

    #[test]
    fn wall_ledger_ortho_tile_width_invariant_across_screen() {
        let h = 1600.0;
        let w = 2560.0;
        let cam = wall_ledger_camera(h);
        let size_px = 72.0;
        let rotation = TOP_DOWN_TILE_ROTATION;
        let width_at = |px: f32, py: f32| {
            showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
                win_w: w,
                win_h: h,
                cam: &cam,
                preset: TilePreset::Chinese,
                center_px: [px, py, 0.0],
                rotation_xyz_rad: rotation,
                placement_scale: 1.0,
                size_px,
                use_ray_plane: false,
            })
            .width()
        };
        let center = width_at(w * 0.5, h * 0.5);
        let corner = width_at(w * 0.12, h * 0.15);
        assert!((center - corner).abs() < 0.5, "center {center} corner {corner}");
    }
}
