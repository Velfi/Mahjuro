//! Circular progress ring for hold-to-act prompts (shader-based annular arc).

use crate::render::theme::color;
use crate::render::wgpu_renderer::ArcRingQuadInstance;

/// Build one square GPU instance that draws a ring track plus a clockwise
/// progress arc (from top) around `(cx, cy)`.
pub fn hold_prompt_ring(
    cx: f32,
    cy: f32,
    radius: f32,
    thickness: f32,
    progress: f32,
) -> ArcRingQuadInstance {
    let stroke = thickness.max(2.0);
    let half = stroke * 0.5;
    let outer = radius + half;
    let inner_norm = ((radius - half) / outer).clamp(0.0, 0.999);

    ArcRingQuadInstance {
        rect: [cx - outer, cy - outer, outer * 2.0, outer * 2.0],
        fill_color: color::alpha(color::CHAMPAGNE, 0.92),
        track_color: color::alpha(color::STONE, 0.38),
        params: [inner_norm, progress.clamp(0.0, 1.0), 0.0, 0.0],
    }
}
