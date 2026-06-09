//! Frozen Archive grid + [`ItemInspectScene`] turntable on the showcase overlay.

use std::time::Instant;

use crate::render::draw_cmd::{ShowcaseRenderHints, UiFrame};
use crate::scenes::archive;
use crate::scenes::object3d_inspect::ItemInspectOrbitState;
use crate::ui::focus_nav::FocusDir;
use crate::ui::input::UiAction;

use crate::scenes::{BackgroundId, DrawCtx, OverlayRequest, SceneTransition, UpdateCtx};

#[derive(Clone, Copy)]
struct OrbitTargetLerp {
    from: [f32; 3],
    to: [f32; 3],
    elapsed: f32,
}

const ORBIT_TARGET_LERP_DURATION: f32 = 0.16;

#[inline]
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

pub struct ArchiveInspectPresenter {
    pub orbit: ItemInspectOrbitState,
    last_frame: Instant,
    target_lerp: Option<OrbitTargetLerp>,
}

impl ArchiveInspectPresenter {
    pub fn new(orbit: ItemInspectOrbitState) -> Self {
        Self {
            orbit,
            last_frame: Instant::now(),
            target_lerp: None,
        }
    }

    fn apply_target_lerp(&mut self, dt: f32) {
        let Some(mut lerp) = self.target_lerp else {
            return;
        };
        lerp.elapsed += dt.clamp(0.0, 0.10);
        let t = (lerp.elapsed / ORBIT_TARGET_LERP_DURATION).clamp(0.0, 1.0);
        let eased = crate::scenes::object3d_inspect::ease_in_out_cubic(t);
        self.orbit.target_world = lerp3(lerp.from, lerp.to, eased);
        if t >= 1.0 {
            self.orbit.target_world = lerp.to;
            self.target_lerp = None;
        } else {
            self.target_lerp = Some(lerp);
        }
    }

    pub fn render_hints() -> ShowcaseRenderHints {
        ShowcaseRenderHints {
            layout_use_ray_plane_z: false,
            tile_pack_celebration_tonemap: false,
            shop_tonemap_and_lit_mesh_context: false,
            collection_tonemap_context: false,
            modal_relic_staging: false,
            zodiac_celebration_no_shadow: false,
            doc_tile_no_shadow: false,
        }
    }

    pub fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.apply_target_lerp(dt);

        if self.target_lerp.is_none()
            && let Some(collection) = ctx.suspended_collection.as_deref_mut()
        {
            archive::sync_item_inspect_orbit_target(
                &*collection,
                ctx.layout.window_w,
                ctx.layout.window_h,
                ctx.layout,
                ctx.progress,
                ctx.room_gltf_height_scale,
                &mut self.orbit,
            );
        }

        const ORBIT: f32 = 2.4;
        const P_LIM: f32 = 0.52;
        const ZMIN: f32 = 0.42;
        const ZMAX: f32 = 1.0;
        const ZSPD: f32 = 1.25;
        let (sx, sy) = ctx.item_inspect_orbit_stick;
        self.orbit.yaw += sx * ORBIT * dt;
        self.orbit.pitch = (self.orbit.pitch + sy * ORBIT * dt).clamp(-P_LIM, P_LIM);
        self.orbit.zoom =
            (self.orbit.zoom - ctx.item_inspect_zoom_triggers * ZSPD * dt).clamp(ZMIN, ZMAX);
        const WHEEL_ZOOM: f32 = 0.11;
        self.orbit.zoom = (self.orbit.zoom + ctx.scroll_lines * WHEEL_ZOOM).clamp(ZMIN, ZMAX);

        for a in ctx.actions {
            let dir = match a {
                UiAction::FocusUp => Some(FocusDir::Up),
                UiAction::FocusDown => Some(FocusDir::Down),
                UiAction::FocusPrev => Some(FocusDir::Left),
                UiAction::FocusNext => Some(FocusDir::Right),
                _ => None,
            };
            if let Some(dir) = dir {
                if let Some(collection) = ctx.suspended_collection.as_deref_mut()
                    && collection.inspect_cycle_focus(
                        dir,
                        ctx.layout.window_w,
                        ctx.layout.window_h,
                        ctx.progress,
                        ctx.room_gltf_height_scale,
                        ctx.bus,
                    )
                {
                    let from = self.orbit.target_world;
                    archive::sync_item_inspect_orbit_target(
                        &*collection,
                        ctx.layout.window_w,
                        ctx.layout.window_h,
                        ctx.layout,
                        ctx.progress,
                        ctx.room_gltf_height_scale,
                        &mut self.orbit,
                    );
                    let to = self.orbit.target_world;
                    let dx = to[0] - from[0];
                    let dy = to[1] - from[1];
                    let dz = to[2] - from[2];
                    if dx * dx + dy * dy + dz * dz > 1e-6 {
                        self.target_lerp = Some(OrbitTargetLerp {
                            from,
                            to,
                            elapsed: 0.0,
                        });
                        self.apply_target_lerp(dt);
                    }
                }
                continue;
            }
            if crate::scenes::object3d_inspect::item_inspect_overlay_play_sound(*a) {
                if let Some(collection) = ctx.suspended_collection.as_deref() {
                    archive::push_inspect_artifact_sound_if_present(
                        collection,
                        ctx.progress,
                        ctx.bus,
                    );
                }
                continue;
            }
            if crate::scenes::object3d_inspect::item_inspect_overlay_exit(*a) {
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
