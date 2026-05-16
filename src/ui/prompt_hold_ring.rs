//! Circular progress stroke for hold-to-act prompts (axis-aligned quad approximation).

use crate::render::theme::color;
use crate::render::wgpu_renderer::GpuInstance;

/// Push quads drawing a ring track plus a clockwise progress arc (from top)
/// around `(cx, cy)`. Uses stacked bounding boxes of thick chord segments so
/// corners read softly where segments overlap.
pub fn push_hold_prompt_ring(
    out: &mut Vec<GpuInstance>,
    cx: f32,
    cy: f32,
    radius: f32,
    thickness: f32,
    progress: f32,
) {
    let stroke = thickness.max(2.0);
    let half = stroke * 0.5;
    let start = -std::f32::consts::FRAC_PI_2;
    const SEGMENTS: usize = 56;
    let tau = std::f32::consts::TAU;

    let sweep_fill = progress.clamp(0.0, 1.0) * tau;
    let sweep_track = tau - sweep_fill;

    let track = color::alpha(color::STONE, 0.38);
    let fill = color::alpha(color::CHAMPAGNE, 0.92);

    if sweep_track > 1e-3 {
        let n = (((SEGMENTS as f32) * (sweep_track / tau)).ceil() as usize).clamp(4, SEGMENTS);
        push_arc_stroke(
            out,
            cx,
            cy,
            radius,
            half,
            start + sweep_fill,
            sweep_track,
            track,
            n,
        );
    }
    if sweep_fill > 1e-3 {
        let n = (((SEGMENTS as f32) * (sweep_fill / tau)).ceil() as usize).clamp(4, SEGMENTS);
        push_arc_stroke(out, cx, cy, radius, half, start, sweep_fill, fill, n);
    }
}

fn push_arc_stroke(
    out: &mut Vec<GpuInstance>,
    cx: f32,
    cy: f32,
    r: f32,
    half_w: f32,
    start: f32,
    sweep: f32,
    color: [f32; 4],
    segments: usize,
) {
    let n = segments.max(8);
    let d_theta = sweep / n as f32;
    for i in 0..n {
        let a0 = start + i as f32 * d_theta;
        let a1 = start + (i + 1) as f32 * d_theta;
        let x0 = cx + r * a0.cos();
        let y0 = cy + r * a0.sin();
        let x1 = cx + r * a1.cos();
        let y1 = cy + r * a1.sin();
        thick_segment_bbox(out, x0, y0, x1, y1, half_w * 2.0, color);
    }
}

fn thick_segment_bbox(
    out: &mut Vec<GpuInstance>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thickness: f32,
    color: [f32; 4],
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt().max(1e-4);
    let nx = -dy / len * thickness * 0.5;
    let ny = dx / len * thickness * 0.5;
    let xs = [x0 + nx, x0 - nx, x1 + nx, x1 - nx];
    let ys = [y0 + ny, y0 - ny, y1 + ny, y1 - ny];
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for i in 0..4 {
        min_x = min_x.min(xs[i]);
        max_x = max_x.max(xs[i]);
        min_y = min_y.min(ys[i]);
        max_y = max_y.max(ys[i]);
    }
    out.push(GpuInstance {
        rect: [min_x, min_y, max_x - min_x, max_y - min_y],
        color,
        user: 0,
    });
}
