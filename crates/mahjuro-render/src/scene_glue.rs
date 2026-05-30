//! Small camera / layout helpers formerly on the game scene modules.

pub const BOOK_FACE_WIDTH_FRAC: f32 = 0.0315;
/// Cover depth (mesh Z → world Y) ÷ width (mesh X → world X). Table books lie
/// cover-up in the XY plane (Z-up); screen width follows mesh X, so keep this
/// well above 1.0 for a portrait silhouette after perspective foreshortening.
pub const BOOK_FACE_HEIGHT_OVER_WIDTH: f32 = 3.04;
pub const BOOK_SPINE_THICKNESS_MM: f32 = 7.0;

#[inline]
pub fn book_cover_face_extents_xy(window_w: f32, zoom: f32) -> (f32, f32) {
    let face_w = window_w * BOOK_FACE_WIDTH_FRAC * zoom;
    let face_h = face_w * BOOK_FACE_HEIGHT_OVER_WIDTH;
    (face_w, face_h)
}

/// Portrait decal size for the book cover (+Y face: u → mesh X, v → mesh Z).
#[inline]
pub fn book_cover_decal_dimensions() -> (u32, u32) {
    const SHORT_EDGE: u32 = 192;
    let long = (SHORT_EDGE as f32 * BOOK_FACE_HEIGHT_OVER_WIDTH).round() as u32;
    (SHORT_EDGE, long.max(SHORT_EDGE + 1))
}

use crate::draw_cmd::{CameraParams, Object3d};
use crate::table_transform::translate_rot_scale;
use crate::world_space::pixel_to_world;
use glam::Vec3;

pub fn shop_celebration_camera(_w: f32, h: f32, _env_h: f32) -> CameraParams {
    let cs = (h / 1080_f32).max(1e-6);
    const EYE_Y: f32 = -1780.0;
    const EYE_Z: f32 = 620.0;
    const TARGET_Y: f32 = 90.0;
    const TARGET_Z: f32 = 150.0;
    const FOVY_DEG: f32 = 46.0;
    CameraParams {
        eye: [0.0, EYE_Y * cs, EYE_Z * cs],
        target: [0.0, TARGET_Y * cs, TARGET_Z * cs],
        up: [0.0, 0.0, 1.0],
        fovy_deg: FOVY_DEG,
        clip_near: None,
        clip_far: None,
    }
}

pub fn bowl_model_matrix(window_w: f32, window_h: f32, bowl: &Object3d) -> glam::Mat4 {
    let center = pixel_to_world(
        window_w,
        window_h,
        bowl.pos[0],
        bowl.pos[1],
        bowl.pos[2] + bowl.extents[1] * 0.5,
    );
    translate_rot_scale(center, bowl.rotation_matrix(), Vec3::from(bowl.extents))
}
