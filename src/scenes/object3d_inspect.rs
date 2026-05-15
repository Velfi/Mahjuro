//! **Isolated-first Object3d inspect core** — fixed inspect camera, turntable (yaw/pitch) on the
//! subject mesh, three-point fill, and frame flags shared by shop, collection, and future hosts.
//!
//! ## Adding a new inspect host
//!
//! 1. **Backdrop** — Push a solid quad, [`BackgroundId`](crate::scenes::BackgroundId), or your env
//!    (`shop_environment`, etc.) onto [`UiFrame`](crate::render::draw_cmd::UiFrame) first.
//! 2. **Rig** — Pick or construct an [`InspectRig`] (`::shop`, `::collection`, or custom direction /
//!    distance / FOV / [`InspectLightPreset`]).
//! 3. **Pivot** — Build [`ItemInspectOrbitState`] with `target_world` in the same Z-up world space as
//!    your [`Object3d`](crate::render::draw_cmd::Object3d) anchors.
//! 4. **Apply view** — Call [`apply_inspect_view_to_frame`] (sets camera + point lights + a subject
//!    [`SpotLight`](crate::render::wgpu_renderer::SpotLight); optionally shop storeroom lighting overrides).
//! 5. **Meshes** — Push `object3d` / `object3d_batch` for the subject; compose yaw/pitch with
//!    [`prepend_inspect_orbit_subject_rotation`] on the hero mesh so it orbits under the fixed camera.
//! 6. **Overlay** — Shop and collection **only** use the shared
//!    [`Scene::Showcase`](crate::scenes::Scene::Showcase) inspect presenters (never an in-scene inspect mode).
//!    Parent snapshots live in `suspended_*` [`DrawCtx`](crate::scenes::DrawCtx) / [`UpdateCtx`](crate::scenes::UpdateCtx);
//!    the overlay owns input (see [`crate::scenes::ShopInspectPresenter`] / [`crate::scenes::CollectionInspectPresenter`]).
//!
//! For per-frame pivot correction under a moving camera (shop shelves), run your sync before draw and
//! update `orbit.target_world`, then call `apply_inspect_view_to_frame` with the resolved target.

use glam::{Mat3, Mat4, Vec3};

use crate::render::draw_cmd::{CameraParams, Object3d, UiFrame};
use crate::render::table_transform::{mat4_to_euler_xyz_rad, rot_euler_xyz_rad};
use crate::render::theme::color;
use crate::render::wgpu_renderer::{PointLight, SpotLight};

/// Orbit + zoom state for close-up inspection (right stick, triggers, scroll).
#[derive(Clone, Copy, Debug)]
pub struct ItemInspectOrbitState {
    pub target_world: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// Scales camera offset from the item (smaller = closer). Clamped in showcase inspect presenters' `update`.
    pub zoom: f32,
}

/// Key / fill / rim intensity and radius scale (relative to `window_h.max(120)`).
#[derive(Clone, Copy, Debug)]
pub struct InspectLightPreset {
    pub key_i: f32,
    pub fill_i: f32,
    pub rim_i: f32,
    pub key_r: f32,
    pub fill_r: f32,
    pub rim_r: f32,
}

impl InspectLightPreset {
    /// Storeroom inspect turns off GLB punctual; synthetic lights must land in the same HDR band
    /// as `SHOP_INSPECT_LIT_MESH_HDR_LINEAR_MUL` after ACES (see `tile_hdr_tonemap` shop-inspect branch).
    pub const SHOP: Self = Self {
        key_i: 17.5,
        fill_i: 8.2,
        rim_i: 10.0,
        key_r: 1.1,
        fill_r: 0.95,
        rim_r: 0.58,
    };
    pub const COLLECTION: Self = Self {
        key_i: 4.7,
        fill_i: 2.15,
        rim_i: 2.75,
        key_r: 1.05,
        fill_r: 0.9,
        rim_r: 0.52,
    };
}

/// Canonical offset direction (world), eye distance at zoom=1, vertical FOV, and light tuning.
#[derive(Clone, Copy, Debug)]
pub struct InspectRig {
    pub base_dir: Vec3,
    pub base_distance: f32,
    pub fovy_deg: f32,
    pub light_preset: InspectLightPreset,
}

impl InspectRig {
    /// Storeroom close-up: distance scales with [`crate::render::shop_glb::shop_env_world_scale`].
    pub fn shop(window_h: f32, room_gltf_height_scale: f32) -> Self {
        let h = window_h.max(1.0);
        let s = crate::render::shop_glb::shop_env_world_scale(h, room_gltf_height_scale);
        Self {
            base_dir: Vec3::new(0.32_f32, -0.74, 0.59).normalize(),
            base_distance: 0.52 * s,
            fovy_deg: 32.0,
            light_preset: InspectLightPreset::SHOP,
        }
    }

    /// Archive grid close-up: linear in window height.
    pub fn collection(window_h: f32) -> Self {
        let h = window_h.max(1.0);
        Self {
            base_dir: Vec3::new(0.0_f32, -0.90, 0.44).normalize(),
            base_distance: h * 0.78,
            fovy_deg: 38.0,
            light_preset: InspectLightPreset::COLLECTION,
        }
    }
}

/// Environment-specific frame flags applied with inspect lighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectFrameEnv {
    /// Only set camera + `frame.scene_lighting` punctual (collection, isolated inspect).
    Neutral,
    /// Shop storeroom: disable GLB punctual and clear shop/spot light vectors (matches prior inspect).
    ShopStoreroomInspect,
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

/// Close-up inspect camera: offset from pivot along [`InspectRig::base_dir`] with zoom only.
/// Yaw / pitch spin the subject via [`prepend_inspect_orbit_subject_rotation`], not the camera.
pub fn inspect_orbit_camera(ins: &ItemInspectOrbitState, rig: &InspectRig) -> CameraParams {
    let target = Vec3::from_array(ins.target_world);
    let up = [0.0_f32, 0.0, 1.0];

    let v = inspect_orbit_unrotated_offset_vec(ins, rig);
    let new_eye = target + v;
    CameraParams {
        eye: new_eye.to_array(),
        target: ins.target_world,
        up,
        fovy_deg: rig.fovy_deg,
        clip_near: None,
        clip_far: None,
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

#[inline]
fn world_to_point_light_pos(window_w: f32, window_h: f32, world: Vec3) -> [f32; 3] {
    [world.x + window_w * 0.5, window_h * 0.5 - world.y, world.z]
}

/// Three-point rig in inspect space — not the shop lamp, GLB punctual, or [`InspectRig::collection`] defaults.
pub fn inspect_point_lights(
    window_w: f32,
    window_h: f32,
    cam: &CameraParams,
    target_world: [f32; 3],
    preset: InspectLightPreset,
) -> Vec<PointLight> {
    let target = Vec3::from_array(target_world);
    let eye = Vec3::from_array(cam.eye);
    let mut view_dir = eye - target;
    if view_dir.length_squared() < 1e-8 {
        view_dir = Vec3::new(0.0, -1.0, 0.2);
    }
    let view_dir = view_dir.normalize();
    let mut up = Vec3::from_array(cam.up);
    if up.length_squared() < 1e-8 {
        up = Vec3::Z;
    } else {
        up = up.normalize();
    }
    let mut right = view_dir.cross(up);
    if right.length_squared() < 1e-8 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }

    let scale = window_h.max(120.0);
    let key_world = eye - view_dir * (scale * 0.20);
    let fill_world = target - right * (scale * 0.42) + up * (scale * 0.055);
    let rim_world = target - view_dir * (scale * 0.52) + up * (scale * 0.065);

    let InspectLightPreset {
        key_i,
        fill_i,
        rim_i,
        key_r,
        fill_r,
        rim_r,
    } = preset;

    vec![
        PointLight {
            pos: world_to_point_light_pos(window_w, window_h, key_world),
            radius: scale * key_r,
            color: [1.0, 0.94, 0.82],
            intensity: key_i,
        },
        PointLight {
            pos: world_to_point_light_pos(window_w, window_h, fill_world),
            radius: scale * fill_r,
            color: [0.75, 0.86, 1.0],
            intensity: fill_i,
        },
        PointLight {
            pos: world_to_point_light_pos(window_w, window_h, rim_world),
            radius: scale * rim_r,
            color: [1.0, 0.68, 0.5],
            intensity: rim_i,
        },
    ]
}

/// Tight cone aimed at the inspect pivot — pools extra energy on the hero mesh (`lit_mesh` spot loop).
pub fn inspect_subject_spotlight(
    window_w: f32,
    window_h: f32,
    cam: &CameraParams,
    target_world: [f32; 3],
) -> SpotLight {
    let target = Vec3::from_array(target_world);
    let eye = Vec3::from_array(cam.eye);
    let mut view_dir = eye - target;
    if view_dir.length_squared() < 1e-8 {
        view_dir = Vec3::new(0.0, -1.0, 0.2);
    }
    let view_dir = view_dir.normalize();
    let mut up = Vec3::from_array(cam.up);
    if up.length_squared() < 1e-8 {
        up = Vec3::Z;
    } else {
        up = up.normalize();
    }
    let scale = window_h.max(120.0);
    // Between pivot and camera, nudged toward the key side so the cone grazes the front face.
    let light_world = target + view_dir * (scale * 0.34) + up * (scale * 0.07);
    let mut dir_w = target - light_world;
    if dir_w.length_squared() < 1e-8 {
        dir_w = -view_dir;
    } else {
        dir_w = dir_w.normalize();
    }
    let cos_outer = (32.0_f32).to_radians().cos();
    let cos_inner = (18.0_f32).to_radians().cos();
    SpotLight {
        pos: world_to_point_light_pos(window_w, window_h, light_world),
        dir: dir_w.to_array(),
        radius: scale * 1.5,
        cos_outer,
        cos_inner,
        color: color::rgb(color::PARCHMENT),
        intensity: 15.0,
    }
}

/// Sets inspect camera, synthetic point lights, and optional shop storeroom lighting overrides.
pub fn apply_inspect_view_to_frame(
    frame: &mut UiFrame,
    window_w: f32,
    window_h: f32,
    orbit: &ItemInspectOrbitState,
    rig: &InspectRig,
    target_world: [f32; 3],
    env: InspectFrameEnv,
) {
    let cam = inspect_orbit_camera(orbit, rig);
    frame.camera_override = Some(cam);
    frame.scene_lighting.set_smooth_points(inspect_point_lights(
        window_w,
        window_h,
        &cam,
        target_world,
        rig.light_preset,
    ));
    frame.scene_lighting.spot_lights = vec![inspect_subject_spotlight(
        window_w,
        window_h,
        &cam,
        target_world,
    )];

    if env == InspectFrameEnv::ShopStoreroomInspect {
        frame.scene_lighting.embedded_gltf_punctual = false;
    }
}
