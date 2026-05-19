//! Score-panel layout anchors for the gameplay HUD. The readout is **2D**
//! (screen-space text in `scene_behavior`); this module only shares pixel/world
//! anchors so cascade HUD glyphs and streaming score popups stay aligned with
//! the same panel geometry.
//!
//! [`crate::ui::layout::LayoutResult::mm`] is still the right source for any
//! world-unit lifts that must match hand tiles — do not multiply raw
//! [`crate::ui::layout::Rect::w`] / `h` (pixels) into world extents.

use crate::render::world_space::LayoutAnchorPx;
use crate::ui::layout::{LayoutResult, Rect};
use crate::ui::scene_layout::GameplayPositions;

/// Vertical placement hint for cascade / 3D anchors relative to the score strip.
/// The **2D** readout uses its own centered rect (see `scene_behavior`); this only
/// keeps `reel` / `cascade_pad` aligned with the strip, not the inner text box.
pub mod readout_2d {
    /// Fraction down the score strip for the fly-to anchor (center of thin band).
    pub const ANCHOR_Y_FRAC: f32 = 0.5;
}

/// Shared anchors for cascade hand-off HUD glyphs and streaming score popups.
#[derive(Clone, Copy, Debug)]
pub struct ScoreCounterLayout {
    pub reel: LayoutAnchorPx,
    pub cascade_pad: LayoutAnchorPx,
    pub glyph_scale: f32,
    pub plaque_w: f32,
}

#[inline]
fn reel_anchor_py(sp: &Rect) -> f32 {
    sp.y + sp.h * readout_2d::ANCHOR_Y_FRAC
}

#[inline]
pub fn score_counter_layout(
    layout: &LayoutResult,
    positions: &GameplayPositions,
) -> ScoreCounterLayout {
    let sp = layout.score_panel;
    let plaque_lift = layout.mm(positions.plaque.lift_mm);
    let screen = layout.window_w.min(layout.window_h);
    ScoreCounterLayout {
        reel: LayoutAnchorPx {
            px: sp.x + sp.w * 0.5,
            py: reel_anchor_py(&sp),
            lift_z: plaque_lift * 1.08,
        },
        cascade_pad: LayoutAnchorPx {
            px: sp.x + sp.w * 0.5,
            py: sp.y + sp.h * 1.05,
            lift_z: plaque_lift * 0.6,
        },
        glyph_scale: (screen / 1080.0) * 180.0,
        plaque_w: sp.w,
    }
}
