//! Full-screen item inspect: orbit camera + zoom over a 3D target, as a
//! pushdown overlay so the same mode can be entered from the shop, collection,
//! or other parent scenes. The parent scene is suspended; drawing reuses its
//! frame path via [`DrawCtx::suspended_shop`] or [`DrawCtx::suspended_collection`].

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

/// Apply orbit pitch/yaw and zoom around `ins.target_world`, starting from `base` eye/target.
pub fn inspect_orbit_camera(base: &CameraParams, ins: &ItemInspectOrbitState) -> CameraParams {
    let target = Vec3::from_array(ins.target_world);
    let eye0 = Vec3::from_array(base.eye);
    let mut v = eye0 - target;
    if v.length_squared() < 1e-4 {
        let scale = eye0.y.abs().max(200.0);
        v = Vec3::new(0.0, -scale * 0.9, scale * 0.38);
    }
    v *= ins.zoom;
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
        target: target.to_array(),
        up: base.up,
        fovy_deg: base.fovy_deg,
    }
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
        self.orbit.zoom = (self.orbit.zoom - ctx.shop_inspect_zoom_triggers * ZSPD * dt)
            .clamp(ZMIN, ZMAX);
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
