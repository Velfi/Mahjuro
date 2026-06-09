//! Defeat-screen 3D tableau: low wood floor receding into black + memorial talisman.

use crate::core::memorial_talisman::MemorialTalismanKind;
use crate::render::draw_cmd::{CameraParams, DrawCmd, Object3d, Object3dKind, UiFrame};
use crate::render::primitive::{MaterialSpec, MeshId};
use crate::render::table_transform::rot_fixed_axes_deg;
use crate::render::theme::color;
use crate::render::wgpu_renderer::PointLight;
use crate::render::world_space::{
    object3d_pos_for_screen_at_world_z, object3d_pos_triple_for_world_center, pixel_to_world,
    world_on_camera_ray_plane_z,
};
use crate::ui::placement::{Placement, PlacementAnchor};
use glam::Vec3;

/// Screen-center hero placement: lifted so the tilted tablet clears the felt.
const DEFEAT_MEMORIAL_PLACEMENT: Placement = Placement {
    nx: 0.0,
    ny: -0.10,
    lift_mm: 62.0,
    rx_deg: -12.0,
    ry_deg: 0.0,
    rz_deg: 0.0,
};

/// Wood floor anchor — lower third of the frame, pitched toward the camera.
const DEFEAT_WOOD_PLACEMENT: Placement = Placement {
    nx: 0.0,
    ny: 0.34,
    lift_mm: 1.5,
    rx_deg: 8.0,
    ry_deg: 0.0,
    rz_deg: 0.0,
};

const DEFEAT_FOVY_DEG: f32 = 38.0;
const DEFEAT_WOOD_WIDTH_MUL: f32 = 1.65;
const DEFEAT_WOOD_DEPTH_MUL: f32 = 5.0;
const DEFEAT_WOOD_THICKNESS_MUL: f32 = 0.018;
/// Scene key/fill intensities for the black-void memorial tableau.
const DEFEAT_LIGHT_SCALE: f32 = 0.32;

/// Push wood floor + centered, lit memorial talisman (no sunlit-water backdrop).
pub fn push_defeat_memorial_tableau(
    frame: &mut UiFrame,
    layout: &crate::ui::layout::LayoutResult,
    kind: MemorialTalismanKind,
) {
    let w = layout.window_w;
    let h = layout.window_h;

    let anchor = PlacementAnchor::new(
        [w * 0.5, h * 0.5, 0.0],
        rot_fixed_axes_deg(82.0, 0.0, 0.0),
        &DEFEAT_MEMORIAL_PLACEMENT,
        layout,
    );
    let tx = anchor.pos[0];
    let ty = anchor.pos[1];
    let tz = anchor.pos[2];
    let target_w = pixel_to_world(w, h, tx, ty, tz);

    // Camera is world-space; frame the talisman at screen center.
    let cs = (h / 2104.0_f32).max(1e-4);
    let eye = Vec3::new(target_w.x, target_w.y - 980.0 * cs, target_w.z + 440.0 * cs);
    let look_target = Vec3::new(target_w.x, target_w.y, target_w.z + 12.0 * cs);
    let cam = CameraParams {
        eye: eye.to_array(),
        target: look_target.to_array(),
        up: [0.0, 0.0, 1.0],
        fovy_deg: DEFEAT_FOVY_DEG,
        clip_near: Some(0.05),
        clip_far: None,
    };
    frame.camera_override = Some(cam);

    let hero = h.min(w);
    let tscale = hero * 0.13;
    let accent = kind.accent_color();
    let talisman_rotation = anchor.object3d_rotation();

    let wood_anchor = PlacementAnchor::new(
        [w * 0.5, h * 0.5, 0.0],
        glam::Mat4::IDENTITY,
        &DEFEAT_WOOD_PLACEMENT,
        layout,
    );
    let board_px = wood_anchor.pos[0];
    let mut board_py = wood_anchor.pos[1];
    // Nudge toward the bottom edge so the slab reads as a floor receding into the black above.
    board_py += h * 0.08;
    let wood_plane_z = target_w.z * 0.22;
    let wood_world = world_on_camera_ray_plane_z(w, h, &cam, board_px, board_py, wood_plane_z);
    let wood_pos = object3d_pos_for_screen_at_world_z(w, h, &cam, board_px, board_py, wood_plane_z);
    let wood_board = Object3d {
        pos: wood_pos,
        extents: [
            w * DEFEAT_WOOD_WIDTH_MUL,
            h * DEFEAT_WOOD_DEPTH_MUL,
            hero * DEFEAT_WOOD_THICKNESS_MUL,
        ],
        rotation: wood_anchor.object3d_rotation(),
        color: color::darken(color::WALNUT_RAISED, 0.08),
        kind: Object3dKind::Primitive {
            shape: MeshId::Cube,
            material: MaterialSpec::lacquered_wood_flat(),
            pick_id: None,
            silhouette: false,
        },
        hover_target: 0.0,
        anim_id: 0,
    };
    let memorial = Object3d {
        pos: anchor.pos,
        extents: crate::render::talisman_mesh::talisman_object_extents(tscale),
        rotation: talisman_rotation,
        color: [
            (accent[0] * 1.2).min(1.0),
            (accent[1] * 1.2).min(1.0),
            (accent[2] * 1.2).min(1.0),
            1.0,
        ],
        kind: Object3dKind::MemorialTalisman { kind },
        hover_target: 0.0,
        anim_id: 0,
    };
    frame
        .cmds
        .push(DrawCmd::Object3dBatch(vec![wood_board, memorial]));

    let parchment = color::rgb(color::PARCHMENT);
    let accent_rgb = [accent[0], accent[1], accent[2]];
    let wood_fill = wood_world + Vec3::new(0.0, -h * 0.12, h * 0.22);
    frame.scene_lighting.set_smooth_points(vec![
        PointLight {
            pos: object3d_pos_triple_for_world_center(w, h, wood_fill),
            radius: w.max(h) * 2.0,
            color: parchment,
            intensity: 3.4 * DEFEAT_LIGHT_SCALE,
        },
        PointLight {
            pos: [tx, ty - h * 0.05, tz + h * 0.40],
            radius: w.max(h) * 2.4,
            color: parchment,
            intensity: 5.5 * DEFEAT_LIGHT_SCALE,
        },
        PointLight {
            pos: [tx, ty + h * 0.04, tz + h * 0.10],
            radius: w.max(h) * 1.2,
            color: [0.42, 0.38, 0.50],
            intensity: 0.55 * DEFEAT_LIGHT_SCALE,
        },
        PointLight {
            pos: [tx + w * 0.18, ty, tz + h * 0.16],
            radius: w.max(h) * 1.0,
            color: accent_rgb,
            intensity: 3.0 * DEFEAT_LIGHT_SCALE,
        },
        PointLight {
            pos: [tx - w * 0.18, ty, tz + h * 0.16],
            radius: w.max(h) * 1.0,
            color: [
                accent_rgb[0] * 0.55 + 0.28,
                accent_rgb[1] * 0.55 + 0.22,
                accent_rgb[2] * 0.55 + 0.38,
            ],
            intensity: 2.2 * DEFEAT_LIGHT_SCALE,
        },
    ]);
}
