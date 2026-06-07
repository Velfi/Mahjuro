//! Score-panel layout anchors for the gameplay HUD. Cascade HUD glyphs and
//! streaming score popups align with the authored score frame in
//! [`gameplay.glb`](../../../assets/3d/gameplay.glb) (`frame` mesh bounds).
//!
//! [`crate::ui::layout::LayoutResult::mm`] is still the right source for any
//! world-unit lifts that must match hand tiles.

use crate::render::draw_cmd::CameraParams;
use crate::render::gameplay_glb::{self, SCORE_FRAME};
use crate::render::score_roller_layout::{
    collect_score_roller_pivots_doc, score_roller_bank_screen_rect,
};
use crate::render::world_space::LayoutAnchorPx;
use crate::ui::layout::LayoutResult;
use crate::ui::scene_layout::GameplayPositions;

/// Vertical placement hint for cascade / 3D anchors relative to the score frame.
pub mod readout_2d {
    /// Fraction down the projected frame rect for the fly-to anchor.
    pub const ANCHOR_Y_FRAC: f32 = 0.5;
    /// Fraction below the frame bottom for the chips/×/mult pad.
    pub const PAD_Y_FRAC: f32 = 0.05;
}

/// Shared anchors for cascade hand-off HUD glyphs and streaming score popups.
#[derive(Clone, Copy, Debug)]
pub struct ScoreCounterLayout {
    pub reel: LayoutAnchorPx,
    pub cascade_pad: LayoutAnchorPx,
    pub glyph_scale: f32,
    pub plaque_w: f32,
}

/// Combined cascade geometry: score counter plaque + pad anchors.
#[derive(Clone, Copy, Debug)]
pub struct ScoreCascadeLayout {
    pub counter: ScoreCounterLayout,
}

#[inline]
fn glyph_scale_for(layout: &LayoutResult) -> f32 {
    (layout.window_w.min(layout.window_h) / 1080.0) * 180.0
}

fn score_cascade_layout_from_frame_rect(
    layout: &LayoutResult,
    positions: &GameplayPositions,
    frame: [f32; 4],
) -> ScoreCascadeLayout {
    let [fx, fy, fw, fh] = frame;
    let plaque_lift = layout.mm(positions.score_reel_lift_mm);
    let reel_py = fy + fh * readout_2d::ANCHOR_Y_FRAC;
    let pad_py = fy + fh + fh * readout_2d::PAD_Y_FRAC;
    let pad_lift = plaque_lift * 0.6;
    ScoreCascadeLayout {
        counter: ScoreCounterLayout {
            reel: LayoutAnchorPx {
                px: fx + fw * 0.5,
                py: reel_py,
                lift_z: plaque_lift * 1.08,
            },
            cascade_pad: LayoutAnchorPx {
                px: fx + fw * 0.5,
                py: pad_py,
                lift_z: pad_lift,
            },
            glyph_scale: glyph_scale_for(layout),
            plaque_w: fw,
        },
    }
}

/// Resolve cascade anchors from the authored `frame` mesh in `gameplay.glb`.
pub fn resolve_score_cascade_layout(
    layout: &LayoutResult,
    positions: &GameplayPositions,
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_height_scale: f32,
) -> anyhow::Result<ScoreCascadeLayout> {
    gameplay_glb::with_gameplay_glb_cpu(|cpu| {
        let cpu = cpu.ok_or_else(|| anyhow::anyhow!("gameplay.glb not loaded"))?;
        let min_rw = (w * 0.04).max(32.0);
        let min_rh = (h * 0.03).max(16.0);
        let frame = gameplay_glb::gameplay_marker_screen_rect_resolved(
            w,
            h,
            cam,
            env_height_scale,
            cpu,
            SCORE_FRAME,
            min_rw,
            min_rh,
        )?;
        Ok(score_cascade_layout_from_frame_rect(
            layout, positions, frame,
        ))
    })
}

/// Best-effort fly target for score popups: authored `score_pops_lerp_target` empty,
/// then the frame reel anchor, then legacy score-panel center.
pub fn resolve_score_popup_fly_dest(
    layout: &LayoutResult,
    positions: &GameplayPositions,
    env_height_scale: f32,
) -> LayoutAnchorPx {
    if let Some(a) = try_resolve_score_popup_reel_dest(layout, env_height_scale) {
        return a;
    }
    if let Some(cascade) = try_resolve_score_cascade_layout(layout, positions, env_height_scale) {
        return cascade.counter.reel;
    }
    let (px, py) = layout.fallback_score_center();
    LayoutAnchorPx {
        px,
        py,
        lift_z: layout.mm(positions.score_reel_lift_mm) * 1.08,
    }
}

/// Best-effort fly target for score popups: authored `score_pops_lerp_target` empty.
pub fn try_resolve_score_popup_reel_dest(
    layout: &LayoutResult,
    env_height_scale: f32,
) -> Option<LayoutAnchorPx> {
    let cam = gameplay_glb::gameplay_camera_from_glb_if_present(layout.window_h, env_height_scale)?;
    gameplay_glb::with_gameplay_glb_cpu(|cpu| {
        let cpu = cpu?;
        gameplay_glb::gameplay_marker_popup_fly_anchor(
            layout.window_w,
            layout.window_h,
            &cam,
            env_height_scale,
            cpu,
            gameplay_glb::SCORE_POPS_LERP_TARGET,
        )
    })
}

/// Best-effort cascade layout for update paths without a per-frame camera override.
pub fn try_resolve_score_cascade_layout(
    layout: &LayoutResult,
    positions: &GameplayPositions,
    env_height_scale: f32,
) -> Option<ScoreCascadeLayout> {
    let cam = gameplay_glb::gameplay_camera_from_glb_if_present(layout.window_h, env_height_scale)?;
    resolve_score_cascade_layout(
        layout,
        positions,
        layout.window_w,
        layout.window_h,
        &cam,
        env_height_scale,
    )
    .ok()
}

fn split_score_frame_rect([fx, fy, fw, fh]: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let half = fw * 0.5;
    let pad = fw * 0.02;
    (
        [fx + pad, fy, half - pad * 1.5, fh],
        [fx + half + pad * 0.5, fy, half - pad * 1.5, fh],
    )
}

/// Focus rects for the round-score and blind-target odometer banks.
pub fn resolve_score_roller_bank_focus_rects(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_height_scale: f32,
) -> Option<([f32; 4], [f32; 4])> {
    gameplay_glb::with_gameplay_glb_cpu(|cpu| {
        let cpu = cpu?;
        let min_rw = (w * 0.04).max(32.0);
        let min_rh = (h * 0.03).max(16.0);
        let frame = gameplay_glb::gameplay_marker_screen_rect_resolved(
            w,
            h,
            cam,
            env_height_scale,
            cpu,
            SCORE_FRAME,
            min_rw,
            min_rh,
        )
        .ok()?;
        let pivots = collect_score_roller_pivots_doc(cpu);
        let score = score_roller_bank_screen_rect(w, h, cam, env_height_scale, cpu, &pivots, 0);
        let target = score_roller_bank_screen_rect(w, h, cam, env_height_scale, cpu, &pivots, 1);
        Some(match (score, target) {
            (Some(score), Some(target)) => (score, target),
            _ => split_score_frame_rect(frame),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::gameplay_glb::{load_gameplay_glb_from_bytes, validate_gameplay_glb};

    #[test]
    fn score_frame_projects_on_screen() {
        let bytes = include_bytes!("../../../assets/3d/gameplay.glb");
        let cpu = load_gameplay_glb_from_bytes(bytes).expect("gameplay.glb");
        let cpu = validate_gameplay_glb(cpu).expect("validate");
        assert!(
            cpu.marker_mesh_bounds_doc_for(SCORE_FRAME).is_some(),
            "frame mesh bounds required for score cascade layout"
        );
        let w = 1920.0_f32;
        let h = 1080.0_f32;
        let env_h = 1.0_f32;
        let layout = {
            let mut solver = crate::ui::layout::UiLayout::new();
            solver.solve(w, h)
        };
        let positions = GameplayPositions::default();
        let cam = gameplay_glb::gameplay_camera_from_cpu(&cpu, h, env_h).expect("camera");
        let cascade =
            resolve_score_cascade_layout(&layout, &positions, w, h, &cam, env_h).expect("layout");
        let (fallback_x, fallback_y) = layout.fallback_score_center();
        let reel = cascade.counter.reel;
        assert!(
            reel.px.is_finite() && reel.py.is_finite() && reel.lift_z.is_finite(),
            "reel finite"
        );
        assert!(
            reel.px > fallback_x || reel.py < fallback_y,
            "GLB frame reel should differ from fallback score center ({reel:?} vs ({fallback_x}, {fallback_y}))"
        );
        assert!(cascade.counter.plaque_w > 32.0);
    }

    #[test]
    fn score_pops_lerp_target_resolves_on_screen() {
        use crate::render::gameplay_glb::SCORE_POPS_LERP_TARGET;
        use crate::render::world_space::pixel_to_world;
        let bytes = include_bytes!("../../../assets/3d/gameplay.glb");
        let cpu = load_gameplay_glb_from_bytes(bytes).expect("gameplay.glb");
        let cpu = validate_gameplay_glb(cpu).expect("validate");
        assert!(
            cpu.markers.contains_key(SCORE_POPS_LERP_TARGET),
            "score_pops_lerp_target empty required"
        );
        let w = 1920.0_f32;
        let h = 1080.0_f32;
        let env_h = 1.0_f32;
        let layout = {
            let mut solver = crate::ui::layout::UiLayout::new();
            solver.solve(w, h)
        };
        let positions = GameplayPositions::default();
        let cam = gameplay_glb::gameplay_camera_from_cpu(&cpu, h, env_h).expect("camera");
        let dest = resolve_score_popup_fly_dest(&layout, &positions, env_h);
        assert!(dest.px.is_finite() && dest.py.is_finite() && dest.lift_z.is_finite());
        let world = pixel_to_world(w, h, dest.px, dest.py, dest.lift_z);
        let (sx, sy) = cam.project_world_to_screen(w, h, world);
        assert!(
            sx > 0.0 && sx < w && sy > 0.0 && sy < h,
            "lerp target should project on-screen, got ({}, {})",
            sx,
            sy
        );
    }
}
