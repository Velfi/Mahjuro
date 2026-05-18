//! Moths fluttering back and forth around the main-menu door light (`door_light` in `main_menu.glb`).

use crate::render::draw_cmd::{Object3d, Object3dKind, UiFrame};
use crate::render::table_transform::mat4_to_euler_xyz_rad;
use glam::Mat4;

/// Number of 3D insects orbiting a lamp (must be ≤ `MAX_BUG_SLOTS` in the renderer).
pub const BUG_COUNT: usize = 6;

const _: () = assert!(BUG_COUNT <= crate::render::wgpu_renderer::MAX_BUG_SLOTS);

/// Per-bug: (orbit_radius_frac, orbit_z_offset_frac, orbit_speed_rad/s, body_size_frac).
/// Radius and Z offsets are relative to `lamp_h`.
pub const BUG_PARAMS: [(f32, f32, f32, f32); BUG_COUNT] = [
    (0.55, -0.10, 3.60, 0.68),
    (0.80, -0.25, 2.25, 0.75),
    (0.45, -0.05, 5.10, 0.58),
    (0.90, -0.35, 1.65, 0.82),
    (0.60, -0.20, 4.20, 0.65),
    (0.70, -0.15, 2.85, 0.72),
];

pub fn initial_bug_phases() -> [f32; BUG_COUNT] {
    let mut phases = [0.0_f32; BUG_COUNT];
    for (i, p) in phases.iter_mut().enumerate() {
        *p = i as f32 * std::f32::consts::TAU / BUG_COUNT as f32;
    }
    phases
}

pub fn advance_bug_phases(bug_phases: &mut [f32; BUG_COUNT], dt: f32) {
    for (i, phase) in bug_phases.iter_mut().enumerate() {
        *phase = (*phase + BUG_PARAMS[i].2 * dt) % std::f32::consts::TAU;
    }
}

/// Push moth [`Object3d`] bugs swaying horizontally around `lamp_center` (`[px, py, world_z]` anchor).
pub fn push_moths_around_lamp(
    frame: &mut UiFrame,
    w: f32,
    h: f32,
    lamp_center: [f32; 3],
    lamp_w: f32,
    lamp_h: f32,
    t_now: f32,
    bug_phases: &[f32; BUG_COUNT],
) {
    let lp = lamp_center;
    let bulb_wz = lp[2];
    let bulb_wx = lp[0] - w * 0.5;
    let bulb_wy = h * 0.5 - lp[1];
    let bug_body_len = h * 0.012;
    let flap_hz: f32 = 25.0;
    let flap_amp: f32 = 0.82;

    let sample_bug = |i: usize, t_back: f32| -> ([f32; 3], [f32; 3], Mat4, f32) {
        let (r_frac, z_frac, speed, size_frac) = BUG_PARAMS[i];
        let fi = i as f32;
        let t = t_now - t_back;
        let phase = bug_phases[i] - speed * t_back;

        let bob_freq = 2.3 + fi * 0.71;
        let drift_freq = 1.1 + fi * 0.43;
        let pitch_freq = 3.7 + fi * 0.57;

        let bob = (t * bob_freq + fi * 1.3).sin() * lamp_h * 0.15;
        let r_nom = lamp_w * r_frac;
        let r_drift = (t * drift_freq + fi * 2.1).sin() * r_nom * 0.20;
        let bug_wz = bulb_wz + lamp_h * z_frac + bob;

        let wing_half_span = 1.13 * size_frac * bug_body_len;
        let orbit_r =
            (r_nom + r_drift).max(lamp_w * 0.72 + bug_body_len * 0.6 + wing_half_span);

        let bug_wx = bulb_wx + orbit_r * phase.cos();
        let bug_wy = bulb_wy;
        let bug_px = bug_wx + w * 0.5;
        let bug_py = h * 0.5 - bug_wy;
        let bug_sz = bug_body_len * size_frac;

        let tx = -phase.sin();
        let ty = 0.0;
        let bank = std::f32::consts::FRAC_PI_4 * 0.5 + (t * 1.9 + fi * 0.8).sin() * 0.30;
        let pitch = (t * pitch_freq + fi * 0.5).sin() * 0.25;
        let yaw = if tx.abs() > 1e-4 {
            Mat4::from_cols(
                glam::Vec4::new(tx, ty, 0.0, 0.0),
                glam::Vec4::new(-ty, tx, 0.0, 0.0),
                glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
                glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
            )
        } else {
            Mat4::IDENTITY
        };
        let rot = yaw * Mat4::from_rotation_x(bank) * Mat4::from_rotation_y(pitch);
        let flap = flap_amp * (t * flap_hz * std::f32::consts::TAU + fi * 1.3).sin();
        (
            [bug_px, bug_py, bug_wz],
            [bug_sz, bug_sz, bug_sz],
            rot,
            flap,
        )
    };

    let mut bugs: Vec<Object3d> = Vec::with_capacity(BUG_COUNT);
    for i in 0..BUG_COUNT {
        let (pos, extents, rot, flap_rad) = sample_bug(i, 0.0);
        let fi = i as f32;
        let speed_factor = (t_now * flap_hz * std::f32::consts::TAU + fi * 1.3).cos().abs();
        let live_wing_alpha = 0.92 - 0.55 * speed_factor;
        let blur_alpha = 0.32 * speed_factor;
        bugs.push(Object3d {
            pos,
            extents,
            rotation: mat4_to_euler_xyz_rad(rot),
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Bug {
                slot: i,
                flap_rad,
                live_wing_alpha,
                blur_alpha,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });
    }
    frame.object3d_batch(bugs);
}
