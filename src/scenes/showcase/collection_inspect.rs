//! Frozen Archive grid + [`ItemInspectScene`] turntable on the showcase overlay.

use std::time::Instant;

use crate::render::draw_cmd::{ShowcaseRenderHints, UiFrame};
use crate::scenes::object3d_inspect::ItemInspectOrbitState;
use crate::ui::input::UiAction;

use crate::scenes::{BackgroundId, DrawCtx, OverlayRequest, SceneTransition, UpdateCtx};

pub struct CollectionInspectPresenter {
    pub orbit: ItemInspectOrbitState,
    last_frame: Instant,
}

impl CollectionInspectPresenter {
    pub fn new(orbit: ItemInspectOrbitState) -> Self {
        Self {
            orbit,
            last_frame: Instant::now(),
        }
    }

    pub fn render_hints() -> ShowcaseRenderHints {
        ShowcaseRenderHints {
            object3d_use_camera_ray_plane_z: false,
            showcase_tiles_use_camera_ray_plane_z: false,
            tile_pack_celebration_tonemap: false,
            shop_tonemap_and_lit_mesh_context: false,
            collection_tonemap_context: true,
        }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        const ORBIT: f32 = 2.4;
        const P_LIM: f32 = 0.52;
        const ZMIN: f32 = 0.42;
        const ZMAX: f32 = 2.35;
        const ZSPD: f32 = 1.25;
        let (sx, sy) = ctx.item_inspect_orbit_stick;
        self.orbit.yaw += sx * ORBIT * dt;
        self.orbit.pitch = (self.orbit.pitch + sy * ORBIT * dt).clamp(-P_LIM, P_LIM);
        self.orbit.zoom =
            (self.orbit.zoom - ctx.item_inspect_zoom_triggers * ZSPD * dt).clamp(ZMIN, ZMAX);
        const WHEEL_ZOOM: f32 = 0.11;
        self.orbit.zoom = (self.orbit.zoom + ctx.scroll_lines * WHEEL_ZOOM).clamp(ZMIN, ZMAX);

        for a in ctx.actions {
            if matches!(
                a,
                UiAction::NorthFacePress | UiAction::Cancel | UiAction::Pause
            ) {
                *ctx.overlay_request = Some(OverlayRequest::Pop);
                return None;
            }
        }
        None
    }

    pub fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        if let Some(coll) = ctx.suspended_collection {
            let mut frame = coll.draw_collection_frame(ctx, Some(&self.orbit));
            frame.showcase_render_hints = Self::render_hints();
            return frame;
        }
        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.showcase_render_hints = Self::render_hints();
        frame
    }
}
