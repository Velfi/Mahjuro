//! Defeat-screen 3D tableau: centered memorial talisman under a spotlight.

use crate::core::memorial_talisman::MemorialTalismanKind;
use crate::render::draw_cmd::{CameraParams, DrawCmd, Object3d, Object3dKind, UiFrame};
use crate::render::table_transform::rot_fixed_axes_deg;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{PointLight, SpotLight};
use crate::render::world_space::pixel_to_world;
use crate::ui::placement::{Placement, PlacementAnchor};

/// Screen-center hero placement: lifted so the tilted tablet clears the felt.
const DEFEAT_MEMORIAL_PLACEMENT: Placement = Placement {
    nx: 0.0,
    ny: -0.10,
    lift_mm: 62.0,
    rx_deg: -12.0,
    ry_deg: 0.0,
    rz_deg: 0.0,
};

/// Push table + centered, lit memorial talisman (no sunlit-water backdrop).
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
    frame.camera_override = Some(CameraParams {
        eye: [target_w.x, target_w.y - 980.0 * cs, target_w.z + 440.0 * cs],
        target: [target_w.x, target_w.y, target_w.z + 12.0 * cs],
        up: [0.0, 0.0, 1.0],
        fovy_deg: 38.0,
        clip_near: None,
        clip_far: None,
    });

    frame.table();

    let hero = h.min(w);
    let tscale = hero * 0.13;
    let accent = kind.accent_color();
    let memorial = Object3d {
        pos: anchor.pos,
        extents: crate::render::talisman_mesh::talisman_object_extents(tscale),
        rotation: anchor.object3d_rotation(),
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
    frame.cmds.push(DrawCmd::Object3dBatch(vec![memorial]));

    let parchment = color::rgb(color::PARCHMENT);
    let accent_rgb = [accent[0], accent[1], accent[2]];
    frame.scene_lighting.set_smooth_points(vec![
        PointLight {
            pos: [tx, ty - h * 0.05, tz + h * 0.40],
            radius: w.max(h) * 2.4,
            color: parchment,
            intensity: 5.5,
        },
        PointLight {
            pos: [tx, ty + h * 0.04, tz + h * 0.10],
            radius: w.max(h) * 1.2,
            color: [0.42, 0.38, 0.50],
            intensity: 0.55,
        },
        PointLight {
            pos: [tx + w * 0.18, ty, tz + h * 0.16],
            radius: w.max(h) * 1.0,
            color: accent_rgb,
            intensity: 3.0,
        },
        PointLight {
            pos: [tx - w * 0.18, ty, tz + h * 0.16],
            radius: w.max(h) * 1.0,
            color: [
                accent_rgb[0] * 0.55 + 0.28,
                accent_rgb[1] * 0.55 + 0.22,
                accent_rgb[2] * 0.55 + 0.38,
            ],
            intensity: 2.2,
        },
    ]);

    let cos_outer = 30.0_f32.to_radians().cos();
    let cos_inner = 10.0_f32.to_radians().cos();
    let spot_pos = [tx, ty - h * 0.04, tz + h * 0.44];
    let light_w = pixel_to_world(w, h, spot_pos[0], spot_pos[1], spot_pos[2]);
    let mut dir = target_w - light_w;
    if dir.length_squared() < 1e-6 {
        dir = glam::Vec3::new(0.0, 0.38, -1.0);
    } else {
        dir = dir.normalize();
    }
    frame.scene_lighting.spot_lights = vec![SpotLight {
        pos: spot_pos,
        dir: dir.to_array(),
        radius: w.max(h) * 3.0,
        cos_outer,
        cos_inner,
        color: parchment,
        intensity: 12.0,
    }];
}
