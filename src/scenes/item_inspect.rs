//! Full-screen item inspect: [`ItemInspectScene`] is its own [`crate::scenes::Scene`] on the
//! overlay stack (input + pop). It paints by calling shop/collection **module** draw helpers
//! ([`crate::scenes::shop::render_shop_frame`], etc.) with frozen parent state from
//! [`DrawCtx::suspended_shop`] / [`DrawCtx::suspended_collection`].
//!
//! Orbit math and lighting live in [`super::object3d_inspect`].

use std::time::Instant;

use crate::render::draw_cmd::UiFrame;
use crate::ui::input::UiAction;

use super::{BackgroundId, DrawCtx, OverlayRequest, SceneBehavior, SceneTransition, UpdateCtx};
use crate::scenes::shop::{self, render_shop_frame};

pub use super::object3d_inspect::ItemInspectOrbitState;

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
                    return render_shop_frame(shop, ctx, Some(&self.orbit));
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
