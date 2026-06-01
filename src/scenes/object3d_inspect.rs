//! **Isolated-first Object3d inspect core** — fixed inspect camera, turntable (yaw/pitch) on the
//! subject mesh, and frame flags shared by shop, collection, and future hosts.
//!
//! Inspect inherits lighting from the host scene ([`crate::scenes::shop::render_shop_frame`],
//! [`crate::scenes::archive::ArchiveScene::draw_archive_frame`], …).
//!
//! ## Adding a new inspect host
//!
//! 1. **Backdrop** — Push a solid quad, [`BackgroundId`](crate::scenes::BackgroundId), or your env
//!    (`shop_environment`, etc.) onto [`UiFrame`](crate::render::draw_cmd::UiFrame) first.
//! 2. **Rig** — Pick or construct an [`InspectRig`] (`::shop`, `::collection`, or custom direction /
//!    distance / FOV).
//! 3. **Pivot** — Build [`ItemInspectOrbitState`] with `target_world` in the same Z-up world space as
//!    your [`Object3d`](crate::render::draw_cmd::Object3d) anchors.
//! 4. **Apply view** — Lerp [`inspect_orbit_camera`] via [`tick_inspect_dolly`] / [`lerp_camera`].
//! 5. **Meshes** — Push `object3d` / `object3d_batch` for the subject; compose yaw/pitch with
//!    [`prepend_inspect_orbit_subject_rotation`] on the hero mesh so it orbits under the fixed camera.
//! 6. **Overlay** — Collection and shop item inspect use [`Scene::Showcase`](crate::scenes::Scene::Showcase)
//!    presenters with `suspended_*` [`DrawCtx`](crate::scenes::DrawCtx) snapshots; the overlay owns input.
//!    Camera motion uses [`tick_inspect_dolly`] + [`lerp_camera`] to ease between the resting scene camera
//!    and [`inspect_orbit_camera`] (Archive always; shop storeroom since the dolly path landed).
//!
//! Shop storeroom pivot sync uses the **resting** storeroom camera only — shelf anchors are
//! perspective-corrected with `world_on_camera_ray_plane_z`, so re-projecting under the inspect
//! camera while also moving the look target creates a feedback loop (visible tremble).

use std::cell::Cell;
use std::time::Instant;

use glam::{Mat3, Mat4, Vec3};

use crate::render::draw_cmd::{CameraParams, Object3d};
use crate::render::table_transform::{mat4_to_euler_xyz_rad, rot_euler_xyz_rad};
use crate::ui::input::UiAction;

/// Inputs that dismiss a [`crate::scenes::ShowcasePresenter`] item-inspect overlay.
pub const ITEM_INSPECT_OVERLAY_EXIT: [UiAction; 4] = [
    UiAction::Confirm,
    UiAction::NorthFacePress,
    UiAction::Cancel,
    UiAction::Pause,
];

/// Whether `action` should pop shop / archive item inspect on the showcase overlay.
#[inline]
pub fn item_inspect_overlay_exit(action: UiAction) -> bool {
    ITEM_INSPECT_OVERLAY_EXIT.contains(&action)
}

/// Orbit + zoom state for close-up inspection (right stick, triggers, scroll).
#[derive(Clone, Copy, Debug)]
pub struct ItemInspectOrbitState {
    pub target_world: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// Scales camera offset from the item (smaller = closer). Clamped in showcase inspect presenters' `update`.
    pub zoom: f32,
}

/// Canonical offset direction (world), eye distance at zoom=1, and vertical FOV.
#[derive(Clone, Copy, Debug)]
pub struct InspectRig {
    pub base_dir: Vec3,
    pub base_distance: f32,
    pub fovy_deg: f32,
}

impl InspectRig {
    /// Storeroom close-up: distance scales with [`crate::render::room_glb::room_env_world_scale`].
    pub fn shop(window_h: f32, room_gltf_height_scale: f32) -> Self {
        let h = window_h.max(1.0);
        let s = crate::render::room_glb::room_env_world_scale(h, room_gltf_height_scale);
        Self {
            base_dir: Vec3::new(0.0_f32, -0.944, 0.330).normalize(),
            base_distance: 0.52 * s,
            fovy_deg: 32.0,
        }
    }

    /// Archive grid close-up: linear in window height.
    pub fn collection(window_h: f32) -> Self {
        let h = window_h.max(1.0);
        Self {
            base_dir: Vec3::new(0.0_f32, -0.90, 0.44).normalize(),
            base_distance: h * 1.17,
            fovy_deg: 38.0,
        }
    }
}

#[inline]
fn inspect_orbit_unrotated_offset_vec(ins: &ItemInspectOrbitState, rig: &InspectRig) -> Vec3 {
    let dir0 = rig.base_dir.normalize_or_zero();
    let dir0 = if dir0.length_squared() < 1e-8 {
        Vec3::new(0.0, -1.0, 0.2).normalize()
    } else {
        dir0
    };
    dir0 * rig.base_distance * ins.zoom
}

/// `rot_p * rot_z` used historically to swing the inspect camera offset around the pivot — now the
/// inverse is applied to the subject mesh while the camera stays on the unrotated offset.
fn inspect_orbit_offset_mat3(ins: &ItemInspectOrbitState, rig: &InspectRig) -> Mat3 {
    let v = inspect_orbit_unrotated_offset_vec(ins, rig);
    let rot_z = Mat3::from_axis_angle(Vec3::Z, ins.yaw);
    let vp = rot_z * v;
    let horiz = Vec3::new(vp.x, vp.y, 0.0);
    let pitch_axis = if horiz.length_squared() < 1e-6 {
        Vec3::X
    } else {
        let hn = horiz.normalize();
        Vec3::new(-hn.y, hn.x, 0.0)
    };
    let rot_p = Mat3::from_axis_angle(pitch_axis, ins.pitch);
    rot_p * rot_z
}

/// Near/far for close-up inspect — avoids the default `far = window_h × 32` band that shimmers
/// when the eye is only a few hundred world units from the subject (shop storeroom scale).
#[inline]
fn inspect_orbit_clip_planes(eye_to_target_dist: f32) -> (f32, f32) {
    let d = eye_to_target_dist.max(8.0);
    let near = (d * 0.04).clamp(6.0, d * 0.8);
    let far = (d * 10.0).max(near + d * 3.0);
    (near, far)
}

/// Swing `cam`'s eye around `pivot` (yaw about +Z, then pitch about the horizontal axis).
pub fn orbit_camera_around_pivot(
    cam: &CameraParams,
    pivot: [f32; 3],
    yaw: f32,
    pitch: f32,
) -> CameraParams {
    let pivot_v = Vec3::from_array(pivot);
    let mut offset = Vec3::from_array(cam.eye) - pivot_v;
    let rot_z = Mat3::from_axis_angle(Vec3::Z, yaw);
    offset = rot_z * offset;
    let horiz = Vec3::new(offset.x, offset.y, 0.0);
    let pitch_axis = if horiz.length_squared() < 1e-6 {
        Vec3::X
    } else {
        let hn = horiz.normalize();
        Vec3::new(-hn.y, hn.x, 0.0)
    };
    let rot_p = Mat3::from_axis_angle(pitch_axis, pitch);
    offset = rot_p * offset;
    CameraParams {
        eye: (pivot_v + offset).to_array(),
        ..*cam
    }
}

/// Close-up inspect camera: offset from pivot along [`InspectRig::base_dir`] with zoom only.
/// Yaw / pitch spin the subject via [`prepend_inspect_orbit_subject_rotation`], not the camera.
pub fn inspect_orbit_camera(ins: &ItemInspectOrbitState, rig: &InspectRig) -> CameraParams {
    let target = Vec3::from_array(ins.target_world);
    let up = [0.0_f32, 0.0, 1.0];

    let v = inspect_orbit_unrotated_offset_vec(ins, rig);
    let new_eye = target + v;
    let (clip_near, clip_far) = inspect_orbit_clip_planes(v.length());
    CameraParams {
        eye: new_eye.to_array(),
        target: ins.target_world,
        up,
        fovy_deg: rig.fovy_deg,
        clip_near: Some(clip_near),
        clip_far: Some(clip_far),
    }
}

/// Left-multiplies the inspect inverse-orbit rotation into [`Object3d::rotation`] (same composition
/// as the renderer’s `translate_rot_scale` path).
#[inline]
pub fn prepend_inspect_orbit_subject_rotation(
    mut obj: Object3d,
    ins: &ItemInspectOrbitState,
    rig: &InspectRig,
) -> Object3d {
    let r = inspect_orbit_offset_mat3(ins, rig);
    let r_inv = r.transpose();
    let base = rot_euler_xyz_rad(obj.rotation[0], obj.rotation[1], obj.rotation[2]);
    let combined = Mat4::from_mat3(r_inv) * base;
    obj.rotation = mat4_to_euler_xyz_rad(combined);
    obj
}

// ── Inspect camera dolly (Archive grid, shop storeroom) ─────────────────────

/// Linear phase state for easing between a resting camera and orbit inspect.
#[derive(Clone, Copy)]
pub struct InspectDolly {
    pub phase: f32,
    pub last_tick: Instant,
}

/// Wall-clock duration for [`tick_inspect_dolly`] to travel `0 ↔ 1`.
pub const INSPECT_DOLLY_DURATION: f32 = 0.34;

/// Cubic ease-in-out on t ∈ [0,1].
pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let u = -2.0 * t + 2.0;
        1.0 - u * u * u * 0.5
    }
}

/// Component-wise lerp between two [`CameraParams`]. Up vector is taken from `a`.
///
/// Near/far use [`CameraParams::clip_planes`] per endpoint (so `None` matches the renderer’s
/// defaults) and interpolate numerically. This avoids sticking to one side’s clip when the other
/// has `None` — e.g. shop storeroom room-tight planes with an inspect camera that omits clips.
pub fn lerp_camera(a: &CameraParams, b: &CameraParams, t: f32, window_h: f32) -> CameraParams {
    fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    }
    let h = window_h.max(1e-6);
    let (near_a, far_a) = a.clip_planes(h);
    let (near_b, far_b) = b.clip_planes(h);
    let near = near_a + (near_b - near_a) * t;
    let far = far_a + (far_b - far_a) * t;
    CameraParams {
        eye: lerp3(a.eye, b.eye, t),
        target: lerp3(a.target, b.target, t),
        up: a.up,
        fovy_deg: a.fovy_deg + (b.fovy_deg - a.fovy_deg) * t,
        clip_near: Some(near),
        clip_far: Some(far),
    }
}

/// Advance `cell`'s dolly toward `target_phase` (`1.0` while inspect is active, `0.0` after pop)
/// and return the eased blend factor used to lerp cameras. `dt` is clamped so a stalled frame cannot
/// teleport the phase.
pub fn tick_inspect_dolly(cell: &Cell<InspectDolly>, target_phase: f32) -> f32 {
    let now = Instant::now();
    let mut anim = cell.get();
    let dt = now
        .saturating_duration_since(anim.last_tick)
        .as_secs_f32()
        .min(0.10);
    anim.last_tick = now;
    let delta = target_phase - anim.phase;
    if delta.abs() > 0.0 {
        let step = (dt / INSPECT_DOLLY_DURATION).max(0.0);
        anim.phase = (anim.phase + delta.signum() * step).clamp(0.0, 1.0);
        if (anim.phase - target_phase).abs() < 1e-4 {
            anim.phase = target_phase;
        }
    }
    cell.set(anim);
    ease_in_out_cubic(anim.phase)
}
