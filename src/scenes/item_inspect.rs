//! Full-screen item inspect: dedicated orbit camera, zoom, and a three-point
//! light rig (`item_inspect_point_lights`), as a pushdown overlay from shop or
//! collection. The parent scene is suspended; mesh draws still reuse
//! [`DrawCtx::suspended_shop`] / [`DrawCtx::suspended_collection`].

use std::time::Instant;

use glam::{Mat3, Vec3};

use crate::render::draw_cmd::{CameraParams, UiFrame};
use crate::ui::input::UiAction;

use super::{BackgroundId, DrawCtx, OverlayRequest, SceneBehavior, SceneTransition, UpdateCtx};
use crate::scenes::shop;

/// Orbit + zoom state for close-up inspection (right stick, triggers, scroll).
#[derive(Clone, Copy, Debug)]
pub struct ItemInspectOrbitState {
    pub target_world: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// Scales camera offset from the item (smaller = closer). Clamped in `update`.
    pub zoom: f32,
}

/// Which parent context this inspect session was opened from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemInspectHost {
    Shop,
    Collection,
}

/// Pushdown scene: orbit the camera around a world-space target.
pub struct ItemInspectScene {
    pub host: ItemInspectHost,
    pub orbit: ItemInspectOrbitState,
    last_frame: Instant,
}

impl ItemInspectScene {
    pub fn new(host: ItemInspectHost, orbit: ItemInspectOrbitState) -> Self {
        Self {
            host,
            orbit,
            last_frame: Instant::now(),
        }
    }
}

/// Close-up inspect rig: canonical offset from the pivot, then yaw (world +Z) / pitch / zoom.
/// Does not inherit the parent scene wide-shot camera.
pub fn item_inspect_orbit_camera(
    host: ItemInspectHost,
    window_h: f32,
    ins: &ItemInspectOrbitState,
    shop_env_height_scale: Option<f32>,
) -> CameraParams {
    let target = Vec3::from_array(ins.target_world);
    let up = [0.0_f32, 0.0, 1.0];

    let h = window_h.max(1.0);
    let (dir0, base_dist, fovy_deg) = match host {
        ItemInspectHost::Shop => {
            let env_h =
                shop_env_height_scale.unwrap_or(crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE);
            let s = crate::render::shop_glb::shop_env_world_scale(h, env_h);
            let dir = Vec3::new(0.32_f32, -0.74, 0.59).normalize();
            let dist = h * 0.52 * s;
            (dir, dist, 32.0_f32)
        }
        ItemInspectHost::Collection => {
            let dir = Vec3::new(0.0_f32, -0.90, 0.44).normalize();
            let dist = h * 0.78;
            (dir, dist, 38.0_f32)
        }
    };

    let v = dir0 * base_dist * ins.zoom;
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
    let vf = rot_p * vp;
    let new_eye = target + vf;
    CameraParams {
        eye: new_eye.to_array(),
        target: ins.target_world,
        up,
        fovy_deg,
    }
}

#[inline]
fn world_to_point_light_pos(window_w: f32, window_h: f32, world: Vec3) -> [f32; 3] {
    [world.x + window_w * 0.5, window_h * 0.5 - world.y, world.z]
}

/// Key / fill / rim for inspect — not the shop lamp, GLB punctual, or collection corridor rig.
pub fn item_inspect_point_lights(
    host: ItemInspectHost,
    window_w: f32,
    window_h: f32,
    cam: &CameraParams,
    target_world: [f32; 3],
) -> Vec<crate::render::wgpu_renderer::PointLight> {
    use crate::render::wgpu_renderer::PointLight;

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

    let (key_i, fill_i, rim_i, key_r, fill_r, rim_r) = match host {
        ItemInspectHost::Shop => (5.5_f32, 2.6, 3.2, 1.1, 0.95, 0.58),
        ItemInspectHost::Collection => (4.7_f32, 2.15, 2.75, 1.05, 0.9, 0.52),
    };

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

impl SceneBehavior for ItemInspectScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if matches!(self.host, ItemInspectHost::Shop) {
            if let Some(shop) = ctx.suspended_shop {
                shop::sync_item_inspect_orbit_target(
                    shop,
                    ctx.run,
                    ctx.layout.window_w,
                    ctx.layout.window_h,
                    &mut self.orbit,
                );
            }
        }

        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        const ORBIT: f32 = 2.4;
        const P_LIM: f32 = 0.52;
        const ZMIN: f32 = 0.42;
        const ZMAX: f32 = 2.35;
        const ZSPD: f32 = 1.25;
        let (sx, sy) = ctx.shop_inspect_orbit_stick;
        self.orbit.yaw += sx * ORBIT * dt;
        self.orbit.pitch = (self.orbit.pitch + sy * ORBIT * dt).clamp(-P_LIM, P_LIM);
        self.orbit.zoom =
            (self.orbit.zoom - ctx.shop_inspect_zoom_triggers * ZSPD * dt).clamp(ZMIN, ZMAX);
        const WHEEL_ZOOM: f32 = 0.11;
        self.orbit.zoom = (self.orbit.zoom + ctx.scroll_lines * WHEEL_ZOOM).clamp(ZMIN, ZMAX);

        for a in ctx.actions {
            if matches!(
                a,
                UiAction::ShopItemInspectToggle | UiAction::Cancel | UiAction::Pause
            ) {
                *ctx.overlay_request = Some(OverlayRequest::Pop);
                return None;
            }
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        match self.host {
            ItemInspectHost::Shop => {
                if let Some(shop) = ctx.suspended_shop {
                    return shop.draw_shop_frame(ctx, Some(&self.orbit));
                }
            }
            ItemInspectHost::Collection => {
                if let Some(coll) = ctx.suspended_collection {
                    return coll.draw_collection_frame(ctx, Some(&self.orbit));
                }
            }
        }
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame
    }

    fn has_blocking_overlay(&self) -> bool {
        true
    }
}
