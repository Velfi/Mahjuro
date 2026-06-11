//! Shared clipped chart helpers for Chronicle and other immediate-mode UI.
//!
//! Follows [Duke Library chart dos and don'ts](https://guides.library.duke.edu/datavis/topten):
//! bar charts use a **zero baseline** and linear height; line/sparkline series may
//! autoscale; keep ≤6 categorical colors from [`color::chart`](crate::render::theme::color::chart);
//! precompute comparisons (avg line, numeric labels) instead of asking viewers to do visual math.

use crate::render::decal::{load_mono_font, load_ui_font, measure_label_advances};
use crate::render::theme::color;
use crate::render::vocabulary_colors::GlossaryMode;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::clip::intersect_rect;
use crate::ui::styled_text::push_colored_line_clipped;

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

pub fn push_quad_clipped(out: &mut Vec<GpuInstance>, rect: [f32; 4], clip: ChartClip, c: [f32; 4]) {
    if let Some(clipped) = chart_clip_rect(rect, clip) {
        push_quad(out, clipped, c);
    }
}

pub fn push_squircle_quad_clipped(
    out: &mut Vec<GpuInstance>,
    rect: [f32; 4],
    clip: ChartClip,
    c: [f32; 4],
) {
    if let Some(clipped) = chart_clip_rect(rect, clip) {
        push_quad(out, clipped, c);
    }
}

pub fn push_label_clipped(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    clip: ChartClip,
    mut label: TextLabel,
) {
    if let Some(clipped) = chart_clip_rect(rect, clip) {
        label.clip_rect = Some(clipped);
        out.push(label);
    }
}

/// Intersect `rect` with a [`ChartClip`] band (shared by quads and labels).
pub fn chart_clip_rect(rect: [f32; 4], clip: ChartClip) -> Option<[f32; 4]> {
    intersect_rect(
        rect,
        [
            rect[0],
            clip.top,
            rect[2],
            (clip.bottom - clip.top).max(0.0),
        ],
    )
}

/// Walnut ledger card chrome shared by Chronicle KPI tiles, section cards, and insight columns.
#[derive(Clone, Copy, Debug)]
pub struct LedgerPanelStyle {
    pub fill: [f32; 4],
    pub top_rule: Option<[f32; 4]>,
    pub bracket: [f32; 4],
    pub bracket_tick: f32,
}

impl LedgerPanelStyle {
    pub const KPI: Self = Self {
        fill: color::alpha(color::WALNUT_INK, 0.38),
        top_rule: None,
        bracket: color::alpha(color::BRASS, 0.48),
        bracket_tick: 7.0,
    };
    pub const INSIGHT: Self = Self {
        fill: color::alpha(color::WALNUT_INK, 0.32),
        top_rule: None,
        bracket: color::alpha(color::BRASS, 0.48),
        bracket_tick: 7.0,
    };
    pub const SECTION: Self = Self {
        fill: color::alpha(color::WALNUT_INK, 0.52),
        top_rule: Some(color::alpha(color::BRASS, 0.38)),
        bracket: color::alpha(color::BRASS, 0.55),
        bracket_tick: 6.0,
    };
}

/// Worn gold corner brackets — lighter than a full card border.
pub fn push_corner_brackets_clipped(
    quads: &mut Vec<GpuInstance>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tick: f32,
    c: [f32; 4],
) {
    for &(dx, dy) in &[
        (0.0, 0.0),
        (w - tick, 0.0),
        (0.0, h - tick),
        (w - tick, h - tick),
    ] {
        push_quad_clipped(quads, [x + dx, y + dy, tick, 1.0], clip, c);
        push_quad_clipped(quads, [x + dx, y + dy, 1.0, tick], clip, c);
    }
}

/// Walnut fill, optional brass top rule, and corner brackets — Chronicle ledger card chrome.
pub fn push_ledger_panel_clipped(
    quads: &mut Vec<GpuInstance>,
    clip: ChartClip,
    rect: [f32; 4],
    style: LedgerPanelStyle,
) {
    let [x, y, w, h] = rect;
    push_quad_clipped(quads, rect, clip, style.fill);
    if let Some(rule) = style.top_rule {
        push_quad_clipped(quads, [x, y, w, 1.0], clip, rule);
    }
    push_corner_brackets_clipped(quads, clip, x, y, w, h, style.bracket_tick, style.bracket);
}

/// Four-edge border stroke (focus rings, pane highlights).
pub fn push_rect_border(out: &mut Vec<GpuInstance>, rect: [f32; 4], thickness: f32, c: [f32; 4]) {
    let [x, y, w, h] = rect;
    let b = thickness;
    push_quad(out, [x, y, w, b], c);
    push_quad(out, [x, y + h - b, w, b], c);
    push_quad(out, [x, y, b, h], c);
    push_quad(out, [x + w - b, y, b, h], c);
}

/// Glossary-tinted single-line label inside a chart pane clip.
pub fn push_colored_label_clipped(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    clip: ChartClip,
    text: &str,
    default: [f32; 4],
    font_px: f32,
    align: TextAlign,
    mono: bool,
) {
    push_colored_line_clipped(
        out,
        rect,
        chart_clip_rect(rect, clip),
        text,
        default,
        font_px,
        align,
        mono,
        GlossaryMode::Prose,
    );
}

/// Tight gutter width from the widest rendered tick label.
pub fn chart_y_axis_width_for_max(max_value: u64, micro_px: f32) -> f32 {
    let max_v = max_value.max(1);
    let axis_font = micro_px;
    let mut widest = pill_label_width(&format_chart_axis_tick(max_v), axis_font);
    for top_frac in [0.0_f32, 0.25, 0.5, 0.75] {
        let value_frac = 1.0 - top_frac;
        let tick_v = (max_v as f64 * value_frac as f64).round() as u64;
        widest = widest.max(pill_label_width(&format_chart_axis_tick(tick_v), axis_font));
    }
    widest + 4.0
}

/// Compact score label for the Y-axis (fits narrow axis gutters).
pub fn format_chart_axis_tick(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        if m >= 10.0 {
            format!("{m:.0}M")
        } else {
            format!("{m:.1}M")
        }
    } else if n >= 1_000 {
        let k = n as f64 / 1_000.0;
        if k >= 10.0 {
            format!("{k:.0}k")
        } else {
            format!("{k:.1}k")
        }
    } else {
        n.to_string()
    }
}

/// Zero-based Y ticks and optional faint horizontal guides for a vertical bar plot.
pub fn push_chart_y_axis(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    axis_x: f32,
    axis_w: f32,
    plot_x: f32,
    plot_w: f32,
    plot_y: f32,
    plot_h: f32,
    max_value: u64,
    micro_px: f32,
    grid_color: [f32; 4],
    draw_grid: bool,
    label_align: TextAlign,
) {
    let max_v = max_value.max(1);
    let axis_font = micro_px;
    let axis_line_h = axis_font + 2.0;
    let tick_gap = axis_line_h + 4.0;
    push_quad_clipped(
        quads,
        [plot_x - 1.0, plot_y, 1.0, plot_h],
        clip,
        color::alpha(color::ANTIQUE, 0.55),
    );
    let mut placed_bottom = plot_y;
    for top_frac in [0.0_f32, 0.25, 0.5, 0.75] {
        let value_frac = 1.0 - top_frac;
        let tick_v = (max_v as f64 * value_frac as f64).round() as u64;
        let line_y = plot_y + plot_h * top_frac;
        if draw_grid && top_frac > 0.0 {
            push_quad_clipped(quads, [plot_x, line_y, plot_w, 1.0], clip, grid_color);
        }
        let label_y = (line_y - axis_line_h * 0.5).max(plot_y);
        if label_y + axis_line_h > plot_y + plot_h + 1.0 {
            continue;
        }
        if label_y < placed_bottom {
            continue;
        }
        placed_bottom = label_y + axis_line_h + tick_gap;
        push_label_clipped(
            labels,
            [axis_x, label_y, axis_w - 4.0, axis_line_h],
            clip,
            TextLabel {
                rect: [axis_x, label_y, axis_w - 4.0, axis_line_h],
                text: format_chart_axis_tick(tick_v),
                color: color::alpha(color::STONE, 0.82),
                font_px: Some(axis_font),
                align: label_align,
                mono: true,
                ..Default::default()
            },
        );
    }
}

/// Baseline at the bottom of a vertical bar plot (y = 0).
pub fn push_chart_plot_baseline(
    quads: &mut Vec<GpuInstance>,
    clip: ChartClip,
    plot_x: f32,
    plot_y: f32,
    plot_w: f32,
    plot_h: f32,
) {
    push_quad_clipped(
        quads,
        [plot_x, plot_y + plot_h, plot_w, 1.5],
        clip,
        color::alpha(color::ANTIQUE, 0.7),
    );
}

/// Chronological axis captions under a time-series bar chart.
pub fn push_chart_time_axis_labels(
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    plot_x: f32,
    plot_y: f32,
    plot_w: f32,
    plot_h: f32,
    cap_h: f32,
    caption_px: f32,
) {
    push_label_clipped(
        labels,
        [plot_x, plot_y + plot_h + 4.0, plot_w * 0.5, cap_h],
        clip,
        TextLabel {
            rect: [plot_x, plot_y + plot_h + 4.0, plot_w * 0.5, cap_h],
            text: "then".into(),
            color: color::alpha(color::STONE, 0.75),
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    push_label_clipped(
        labels,
        [
            plot_x + plot_w * 0.5,
            plot_y + plot_h + 4.0,
            plot_w * 0.5,
            cap_h,
        ],
        clip,
        TextLabel {
            rect: [
                plot_x + plot_w * 0.5,
                plot_y + plot_h + 4.0,
                plot_w * 0.5,
                cap_h,
            ],
            text: "now".into(),
            color: color::alpha(color::STONE, 0.75),
            font_px: Some(caption_px),
            align: TextAlign::Right,
            ..Default::default()
        },
    );
}

/// Measured advance width for a single-line label at `font_px`.
pub fn measure_text_width(text: &str, font_px: f32, mono: bool) -> f32 {
    let font = if mono {
        load_mono_font().or_else(load_ui_font)
    } else {
        load_ui_font()
    };
    let Some(font) = font else {
        return pill_label_width(text, font_px);
    };
    let h = font_px.max(8.0).round().max(1.0) as u32;
    let (_, _, advances) = measure_label_advances(font, text, 8192, h, Some(font_px));
    advances.iter().sum()
}

/// Trim `text` with an ellipsis when it would exceed `max_w` at `font_px`.
pub fn truncate_text_to_width(text: &str, max_w: f32, font_px: f32, mono: bool) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if measure_text_width(text, font_px, mono) <= max_w {
        return text.to_string();
    }
    const ELLIPSIS: &str = "…";
    if measure_text_width(ELLIPSIS, font_px, mono) > max_w {
        return ELLIPSIS.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for take in (0..chars.len()).rev() {
        let candidate: String = chars.iter().take(take).collect::<String>() + ELLIPSIS;
        if measure_text_width(&candidate, font_px, mono) <= max_w {
            return candidate;
        }
    }
    ELLIPSIS.to_string()
}

/// Heuristic width for a single-line pill or caption at `font_px`.
#[inline]
pub fn pill_label_width(text: &str, font_px: f32) -> f32 {
    let chars = text.chars().count() as f32;
    (font_px * chars * 0.48).max(font_px * 2.2)
}

/// Natural pill width for `label` at `caption_px` inside a `row_h` band (before `max_w` clamp).
#[inline]
pub fn yaku_pill_width(label: &str, caption_px: f32, row_h: f32) -> f32 {
    let pill_h = (row_h * 0.70).clamp(caption_px * 1.05, row_h - 4.0);
    let pad = (row_h - pill_h) * 0.5;
    (pill_label_width(label, caption_px) + pad * 2.0).max(caption_px * 2.4)
}

/// Off-white bone-tablet pill with engraved label. Returns drawn width.
pub fn push_yaku_pill(
    squircle_quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    row_h: f32,
    label: &str,
    max_w: f32,
    pill_face: [f32; 4],
    pill_ink: [f32; 4],
    pill_rim: [f32; 4],
    caption_px: f32,
) -> f32 {
    let pill_h = (row_h * 0.70).clamp(caption_px * 1.05, row_h - 4.0);
    let pad = (row_h - pill_h) * 0.5;
    let pill_w = yaku_pill_width(label, caption_px, row_h).min((max_w - 2.0).max(caption_px * 2.4));
    let pill_y = y + pad;
    push_squircle_quad_clipped(
        squircle_quads,
        [x + 1.0, pill_y + 1.5, pill_w, pill_h],
        clip,
        color::alpha(pill_rim, 0.38),
    );
    push_squircle_quad_clipped(squircle_quads, [x, pill_y, pill_w, pill_h], clip, pill_face);
    push_label_clipped(
        labels,
        [x, pill_y, pill_w, pill_h],
        clip,
        TextLabel {
            rect: [x, pill_y, pill_w, pill_h],
            text: label.into(),
            color: pill_ink,
            font_px: Some(caption_px),
            align: TextAlign::Center,
            ..Default::default()
        },
    );
    pill_w
}

/// One labeled horizontal bar row with an off-white yaku name pill (gameplay bone tablets).
pub fn push_yaku_hbar_row(
    squircle_quads: &mut Vec<GpuInstance>,
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
    pill_face: [f32; 4],
    pill_ink: [f32; 4],
    pill_rim: [f32; 4],
    bar_color: [f32; 4],
    value_color: [f32; 4],
    caption_px: f32,
    body_px: f32,
    suffix: Option<&str>,
) {
    let _pill_w = push_yaku_pill(
        squircle_quads,
        labels,
        clip,
        x,
        y,
        row_h,
        label,
        label_w,
        pill_face,
        pill_ink,
        pill_rim,
        caption_px,
    );

    let max_c = max_count.max(1);
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
    let value_rect = [value_x, y, value_w, row_h];
    if suffix.is_some() {
        push_colored_label_clipped(
            labels,
            value_rect,
            clip,
            &value_text,
            value_color,
            body_px,
            TextAlign::Right,
            true,
        );
    } else {
        push_label_clipped(
            labels,
            value_rect,
            clip,
            TextLabel {
                rect: value_rect,
                text: value_text,
                color: value_color,
                font_px: Some(body_px),
                align: TextAlign::Right,
                mono: true,
                ..Default::default()
            },
        );
    }
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
            mono: true,
            ..Default::default()
        },
    );
}

/// Resample to at most `max_points` via linear interpolation (keeps endpoints).
fn resample_sparkline(samples: &[f32], max_points: usize) -> Vec<f32> {
    let max_points = max_points.max(2);
    if samples.len() <= max_points {
        return samples.to_vec();
    }
    let last = samples.len() - 1;
    (0..max_points)
        .map(|i| {
            let t = i as f32 / (max_points - 1) as f32;
            let idx = t * last as f32;
            let lo = idx.floor() as usize;
            let hi = (lo + 1).min(last);
            let frac = idx - lo as f32;
            samples[lo] * (1.0 - frac) + samples[hi] * frac
        })
        .collect()
}

/// Map samples into `[pad, 1-pad]` with a minimum vertical span so flat series
/// still read as a line, not a smear.
fn autoscale_sparkline(samples: &[f32]) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let min = samples.iter().copied().fold(f32::INFINITY, f32::min);
    let max = samples.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let span = (max - min).max(0.18);
    const PAD: f32 = 0.08;
    samples
        .iter()
        .map(|v| {
            let t = (*v - min) / span;
            PAD + t.clamp(0.0, 1.0) * (1.0 - 2.0 * PAD)
        })
        .collect()
}

fn prepare_sparkline(samples: &[f32], plot_w: f32) -> Vec<f32> {
    let max_pts = ((plot_w / 2.5).floor() as usize).clamp(2, 48);
    autoscale_sparkline(&resample_sparkline(samples, max_pts))
}

fn sparkline_point_xy(
    x: f32,
    w: f32,
    plot_top: f32,
    plot_h: f32,
    n: usize,
    i: usize,
    value: f32,
) -> (f32, f32) {
    let px = if n <= 1 {
        x + w * 0.5
    } else {
        x + i as f32 * (w / (n - 1) as f32)
    };
    let py = plot_top + plot_h * (1.0 - value.clamp(0.0, 1.0));
    (px, py)
}

/// Area fill under the sparkline polyline (baseline → line), sampled along each segment.
fn push_sparkline_area_fill(
    quads: &mut Vec<GpuInstance>,
    clip: ChartClip,
    floor_y: f32,
    points: &[(f32, f32)],
    fill: [f32; 4],
) {
    if points.is_empty() {
        return;
    }
    if points.len() == 1 {
        let (px, py) = points[0];
        let bar_h = (floor_y - py).max(0.0);
        if bar_h >= 0.5 {
            push_quad_clipped(quads, [px - 0.5, py, 1.0, bar_h], clip, fill);
        }
        return;
    }
    const SLICE_W: f32 = 1.0;
    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        let dx = x1 - x0;
        let len = dx.abs();
        if len < 0.01 {
            continue;
        }
        let steps = ((len / SLICE_W).ceil() as i32).max(1);
        let slice_w = (len / steps as f32).max(SLICE_W);
        for s in 0..steps {
            let t = (s as f32 + 0.5) / steps as f32;
            let sx = x0 + dx * t;
            let sy = y0 + (y1 - y0) * t;
            let bar_h = (floor_y - sy).max(0.0);
            if bar_h >= 0.5 {
                push_quad_clipped(quads, [sx - slice_w * 0.5, sy, slice_w, bar_h], clip, fill);
            }
        }
    }
}

fn push_sparkline_line_segment(
    quads: &mut Vec<GpuInstance>,
    clip: ChartClip,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thick: f32,
    color: [f32; 4],
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 {
        push_quad_clipped(
            quads,
            [x0 - thick * 0.5, y0 - thick * 0.5, thick, thick],
            clip,
            color,
        );
        return;
    }
    let steps = (len / 0.75).ceil() as i32;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let px = x0 + dx * t;
        let py = y0 + dy * t;
        push_quad_clipped(
            quads,
            [px - thick * 0.5, py - thick * 0.5, thick, thick],
            clip,
            color,
        );
    }
}

/// Sparkline from normalized 0..1 samples (values may be re-scaled for display).
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
    if samples.is_empty() || w < 2.0 || h < 2.0 {
        return;
    }
    let values = prepare_sparkline(samples, w);
    let n = values.len();
    let plot_top = y + 1.0;
    let floor_y = y + h - 1.0;
    let plot_h = (floor_y - plot_top).max(1.0);

    push_quad_clipped(quads, [x, floor_y, w, 1.0], clip, baseline_color);

    let thick = (h * 0.16).clamp(1.25, 2.25);
    let mut points = Vec::with_capacity(n);
    for (i, &v) in values.iter().enumerate() {
        points.push(sparkline_point_xy(x, w, plot_top, plot_h, n, i, v));
    }

    // Light area fill only — sparklines may truncate Y; bar charts must not.
    let fill = color::alpha(line_color, line_color[3] * 0.2);
    push_sparkline_area_fill(quads, clip, floor_y, &points, fill);

    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        push_sparkline_line_segment(quads, clip, x0, y0, x1, y1, thick, line_color);
    }
    if let Some(&(lx, ly)) = points.last() {
        let cap = thick + 0.5;
        push_quad_clipped(
            quads,
            [lx - cap * 0.5, ly - cap * 0.5, cap, cap],
            clip,
            color::lighten(line_color, 0.12),
        );
    }
}

#[cfg(test)]
mod sparkline_tests {
    use super::*;

    #[test]
    fn autoscale_flat_series_gets_visible_span() {
        let out = autoscale_sparkline(&[0.5, 0.5, 0.5]);
        assert!(out.iter().all(|v| *v >= 0.08 && *v <= 0.92));
        assert!(out.last().unwrap() - out.first().unwrap() < 0.01);
    }

    #[test]
    fn autoscale_trend_uses_full_height_band() {
        let out = autoscale_sparkline(&[0.0, 0.5, 1.0]);
        assert!(*out.first().unwrap() <= 0.1);
        assert!(*out.last().unwrap() >= 0.9);
    }

    #[test]
    fn resample_reduces_point_count() {
        let src: Vec<f32> = (0..20).map(|i| i as f32).collect();
        let out = resample_sparkline(&src, 6);
        assert_eq!(out.len(), 6);
        assert!((out[0] - src[0]).abs() < 0.01);
        assert!((out[5] - src[19]).abs() < 0.5);
    }

    #[test]
    fn sparkline_points_span_full_plot_width() {
        let x = 10.0;
        let w = 100.0;
        let values = [0.2, 0.5, 0.8];
        let n = values.len();
        let (x0, _) = sparkline_point_xy(x, w, 0.0, 10.0, n, 0, values[0]);
        let (x1, _) = sparkline_point_xy(x, w, 0.0, 10.0, n, n - 1, values[n - 1]);
        assert!((x0 - x).abs() < 0.01);
        assert!((x1 - (x + w)).abs() < 0.01);
    }

    #[test]
    fn bar_height_frac_is_linear_from_zero() {
        let max_v = 100_u64;
        let score = 85_u64;
        let frac = (score as f32 / max_v as f32).clamp(0.0, 1.0);
        assert!((frac - 0.85).abs() < 0.001);
        let tall = 95_u64;
        let tall_frac = (tall as f32 / max_v as f32).clamp(0.0, 1.0);
        assert!(tall_frac > frac);
        assert!((tall_frac - frac - 0.1).abs() < 0.001);
    }
}

#[cfg(test)]
mod text_fit_tests {
    use super::*;

    #[test]
    fn truncate_text_to_width_keeps_short_copy() {
        let out = truncate_text_to_width("Peak 77.6 k cp", 240.0, 14.0, true);
        assert_eq!(out, "Peak 77.6 k cp");
    }

    #[test]
    fn truncate_text_to_width_ellipsis_long_ordeal_name() {
        let out = truncate_text_to_width("The Iconoclast", 48.0, 14.0, false);
        assert!(out.ends_with('…'));
        assert!(out.len() < "The Iconoclast".len());
        assert!(measure_text_width(&out, 14.0, false) <= 48.0 + 0.5);
    }
}
