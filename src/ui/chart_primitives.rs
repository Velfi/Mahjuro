//! Shared clipped chart helpers for Chronicle and other immediate-mode UI.

use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::clip::intersect_rect;

#[derive(Clone, Copy, Debug)]
pub struct ChartClip {
    pub top: f32,
    pub bottom: f32,
}

pub fn push_quad(out: &mut Vec<GpuInstance>, rect: [f32; 4], c: [f32; 4]) {
    out.push(GpuInstance {
        rect,
        color: c,
        user: 0,
    });
}

pub fn push_quad_clipped(
    out: &mut Vec<GpuInstance>,
    rect: [f32; 4],
    clip: ChartClip,
    c: [f32; 4],
) {
    let clip_rect = [
        rect[0],
        clip.top,
        rect[2],
        (clip.bottom - clip.top).max(0.0),
    ];
    if let Some(clipped) = intersect_rect(rect, clip_rect) {
        push_quad(out, clipped, c);
    }
}

pub fn push_label_clipped(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    clip: ChartClip,
    mut label: TextLabel,
) {
    let clip_rect = [
        rect[0],
        clip.top,
        rect[2],
        (clip.bottom - clip.top).max(0.0),
    ];
    if let Some(clipped) = intersect_rect(rect, clip_rect) {
        label.clip_rect = Some(clipped);
        out.push(label);
    }
}

/// Vertical bar chart; `values` normalized against `max_value`.
pub fn push_vbar_chart(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    values: &[u64],
    max_value: u64,
    bar_color: [f32; 4],
    highlight_last: bool,
    grid_color: [f32; 4],
    caption_px: f32,
) {
    if values.is_empty() {
        return;
    }
    let max_v = max_value.max(1);
    let bn = values.len().max(1);
    let slot = w / bn as f32;
    let col_w = (slot - 4.0).max(3.0).min(slot * 0.85);

    for frac in [0.25_f32, 0.5, 0.75] {
        push_quad_clipped(
            quads,
            [x, y + h * frac, w, 1.0],
            clip,
            grid_color,
        );
    }
    for (i, pk) in values.iter().enumerate() {
        let bx = x + i as f32 * slot + (slot - col_w) * 0.5;
        let frac = *pk as f32 / max_v as f32;
        let bar_h = (h * frac).max(3.0);
        let y0 = y + h - bar_h;
        let is_last = highlight_last && i + 1 == values.len();
        let c = if is_last {
            color::alpha(color::GOLD, 0.92)
        } else {
            bar_color
        };
        push_quad_clipped(quads, [bx, y0, col_w, bar_h], clip, c);
        push_quad_clipped(
            quads,
            [bx, y0, col_w, 1.25],
            clip,
            color::alpha(color::PARCHMENT, 0.45),
        );
        if is_last {
            let dot = col_w.min(8.0);
            push_quad_clipped(
                quads,
                [
                    bx + (col_w - dot) * 0.5,
                    y0 - dot - 3.0,
                    dot,
                    dot,
                ],
                clip,
                color::alpha(color::CHAMPAGNE, 0.95),
            );
        }
    }
    push_quad_clipped(quads, [x, y + h, w, 1.5], clip, color::alpha(color::ANTIQUE, 0.7));
    let cap_h = (caption_px / 0.55).ceil();
    push_label_clipped(
        labels,
        [x, y + h + 4.0, w * 0.5, cap_h],
        clip,
        TextLabel {
            rect: [x, y + h + 4.0, w * 0.5, cap_h],
            text: "earlier".into(),
            color: color::alpha(color::STONE, 0.75),
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    push_label_clipped(
        labels,
        [x + w * 0.5, y + h + 4.0, w * 0.5, cap_h],
        clip,
        TextLabel {
            rect: [x + w * 0.5, y + h + 4.0, w * 0.5, cap_h],
            text: "later".into(),
            color: color::alpha(color::STONE, 0.75),
            font_px: Some(caption_px),
            align: TextAlign::Right,
            ..Default::default()
        },
    );
}

/// Horizontal stacked bar (left segment + right segment).
pub fn push_stacked_bar(
    quads: &mut Vec<GpuInstance>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    left_frac: f32,
    left_color: [f32; 4],
    right_color: [f32; 4],
    grid_color: [f32; 4],
) {
    let lf = left_frac.clamp(0.0, 1.0);
    push_quad_clipped(quads, [x, y + h * 0.5 - 1.0, w, 1.5], clip, grid_color);
    push_quad_clipped(quads, [x, y, w * lf, h], clip, left_color);
    push_quad_clipped(
        quads,
        [x + w * lf, y, w * (1.0 - lf), h],
        clip,
        right_color,
    );
}

/// One labeled horizontal bar row.
pub fn push_hbar_row(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    label: &str,
    count: u32,
    max_count: u32,
    label_w: f32,
    label_color: [f32; 4],
    bar_color: [f32; 4],
    value_color: [f32; 4],
    caption_px: f32,
    body_px: f32,
    suffix: Option<&str>,
) {
    let max_c = max_count.max(1);
    push_label_clipped(
        labels,
        [x, y, label_w, row_h],
        clip,
        TextLabel {
            rect: [x, y, label_w, row_h],
            text: label.into(),
            color: label_color,
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    let label_gap = 8.0;
    let value_gap = 6.0;
    let value_w = match suffix {
        Some(_) => (body_px * 9.0).max(72.0).min(w * 0.42),
        None => (body_px * 3.0).max(28.0).min(w * 0.22),
    };
    let bar_x0 = x + label_w + label_gap;
    let bar_track_w = (w - label_w - label_gap - value_w - value_gap).max(4.0);
    let bw = (bar_track_w * (count as f32 / max_c as f32)).clamp(4.0, bar_track_w);
    push_quad_clipped(
        quads,
        [bar_x0, y + row_h * 0.18, bw, row_h * 0.64],
        clip,
        bar_color,
    );
    let value_text = match suffix {
        Some(s) => format!("{count}{s}"),
        None => format!("{count}"),
    };
    let value_x = x + w - value_w;
    push_label_clipped(
        labels,
        [value_x, y, value_w, row_h],
        clip,
        TextLabel {
            rect: [value_x, y, value_w, row_h],
            text: value_text,
            color: value_color,
            font_px: Some(body_px),
            align: TextAlign::Right,
            ..Default::default()
        },
    );
}

/// Simple sparkline from normalized 0..1 samples.
pub fn push_sparkline(
    quads: &mut Vec<GpuInstance>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    samples: &[f32],
    line_color: [f32; 4],
    baseline_color: [f32; 4],
) {
    if samples.is_empty() {
        return;
    }
    push_quad_clipped(quads, [x, y + h, w, 1.0], clip, baseline_color);
    let n = samples.len().max(1);
    let step = w / (n.saturating_sub(1).max(1) as f32);
    let thick = 2.0_f32;
    for (i, &s) in samples.iter().enumerate() {
        let sx = x + i as f32 * step;
        let sy = y + h - s.clamp(0.0, 1.0) * h;
        push_quad_clipped(quads, [sx, sy, thick, thick], clip, line_color);
        if i + 1 < samples.len() {
            let nx = x + (i + 1) as f32 * step;
            let ns = samples[i + 1].clamp(0.0, 1.0);
            let ny = y + h - ns * h;
            let dx = nx - sx;
            let dy = ny - sy;
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let seg_w = len;
            let seg_h = thick;
            let mid_x = (sx + nx) * 0.5 - seg_w * 0.5;
            let mid_y = (sy + ny) * 0.5 - seg_h * 0.5;
            push_quad_clipped(quads, [mid_x, mid_y, seg_w, seg_h], clip, line_color);
        }
    }
}
