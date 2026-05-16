//! Scrollable 2D career stats for Archive Chronicle (no 3D folio plaques).
//!
//! Uses the House walnut / brass palette ([`crate::render::theme::color`]) so the
//! panel matches modals, tooltips, and the rest of the game. Career summary lines
//! that used to float over the archive room live here exclusively.

use crate::core::progression::{PlayerProgress, RunOutcome, RunRecord};
use crate::core::yaku::YakuKind;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, GradientQuadInstance, TextAlign, TextLabel};
use crate::scenes::archive_career;

/// Cap score history columns so wide profiles stay legible on TV.
const MAX_SCORE_BUCKETS: usize = 48;

fn serious_runs_chronological(progress: &PlayerProgress) -> Vec<&RunRecord> {
    let mut v: Vec<&RunRecord> = progress
        .run_history
        .iter()
        .filter(|r| !r.tutorial_run)
        .collect();
    v.sort_by_key(|r| r.timestamp_unix);
    v
}

fn bucket_peak_scores(runs: &[&RunRecord]) -> (Vec<u64>, u64) {
    let n = runs.len();
    if n == 0 {
        return (Vec::new(), 1);
    }
    let b = n.min(MAX_SCORE_BUCKETS);
    let mut peak = vec![0u64; b];
    if n <= b {
        for (i, r) in runs.iter().enumerate() {
            peak[i] = r.total_score_earned;
        }
    } else {
        for i in 0..n {
            let bi = ((i as u64 * b as u64) / n as u64) as usize;
            let bi = bi.min(b - 1);
            let sc = runs[i].total_score_earned;
            peak[bi] = peak[bi].max(sc);
        }
    }
    let mx = peak.iter().copied().max().unwrap_or(1).max(1);
    (peak, mx)
}

fn layout_constants(h: f32) -> (f32, f32, f32, f32, f32, f32, f32) {
    let body = typography::size(typography::H36, h);
    let title_px = typography::size(typography::H32, h);
    let line_h = (body / 0.55).ceil() + 4.0;
    let title_h = (title_px / 0.55).ceil() + 4.0;
    let gap = (h * 0.018).max(12.0);
    let chart_h = (h * 0.13).max(96.0);
    let bar_row_h = (h * 0.032).max(24.0);
    (body, title_px, line_h, title_h, gap, chart_h, bar_row_h)
}

/// Vertical space for the career summary block (header, frieze lines, tutorial note).
fn career_section_height(progress: &PlayerProgress, line_h: f32, title_h: f32, gap: f32) -> f32 {
    let line_count = archive_career::career_frieze_lines(progress).len().max(1);
    title_h
        + gap * 0.85
        + line_count as f32 * (line_h + gap * 0.45)
        + line_h * 1.05
        + gap * 1.35
}

/// Inner inset from panel edges — must match [`push_chronicle_dashboard`].
#[inline]
pub fn chronicle_panel_margin(w: f32) -> f32 {
    (w * 0.022).max(14.0)
}

/// Total scrollable document height (px) inside the inner band; must stay in lockstep with
/// [`push_chronicle_dashboard`] `doc_y` advances.
pub fn chronicle_dashboard_content_height(w: f32, h: f32, progress: &PlayerProgress) -> f32 {
    let runs = serious_runs_chronological(progress);
    let scale = metrics::scene_scale(w, h);
    let (_body, _title_px, line_h, title_h, gap, chart_h, bar_row_h) = layout_constants(h);
    let bottom_pad = (scale * 10.0).max(8.0);
    let mut doc_y = gap;
    doc_y += career_section_height(progress, line_h, title_h, gap);
    if runs.is_empty() {
        doc_y += title_h + gap * 0.85 + line_h * 2.4;
        return doc_y + bottom_pad;
    }
    doc_y += line_h * 1.05;
    doc_y += title_h + gap * 0.85;
    doc_y += chart_h + gap * 1.75;
    doc_y += title_h + gap * 0.85;
    doc_y += bar_row_h * 0.88 + gap * 0.55;
    doc_y += line_h + gap * 1.35;
    doc_y += title_h + gap * 0.85;
    let mut yaku: Vec<(YakuKind, u32)> = progress
        .yaku_times_scored
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    yaku.sort_by(|a, b| b.1.cmp(&a.1));
    let n = yaku.len().min(7);
    doc_y += n as f32 * (bar_row_h + 5.0);
    doc_y + bottom_pad
}

/// Max `scroll_y` so the inner viewport can reach the bottom of the dashboard document.
#[inline]
pub fn chronicle_dashboard_scroll_max(
    w: f32,
    h: f32,
    panel_h: f32,
    progress: &PlayerProgress,
) -> f32 {
    let m = chronicle_panel_margin(w);
    let viewport = (panel_h - m * 2.0).max(1.0);
    let content = chronicle_dashboard_content_height(w, h, progress);
    (content - viewport).max(0.0)
}

fn push_quad(out: &mut Vec<GpuInstance>, rect: [f32; 4], color: [f32; 4]) {
    out.push(GpuInstance {
        rect,
        color,
        user: 0,
    });
}

/// Brass corner ticks on the **viewport** panel (not scrolled).
fn push_panel_viewport_frame(
    out: &mut Vec<GpuInstance>,
    px: f32,
    py: f32,
    pw: f32,
    ph: f32,
    w: f32,
    h: f32,
) {
    let t = (14.0 * metrics::scene_scale(w, h)).clamp(9.0, 22.0);
    let th = 2.5_f32.max(h * 0.0028);
    let c = color::alpha(color::BRASS, 0.85);
    push_quad(out, [px, py, t, th], c);
    push_quad(out, [px, py, th, t], c);
    push_quad(out, [px + pw - t, py, t, th], c);
    push_quad(out, [px + pw - th, py, th, t], c);
    push_quad(out, [px, py + ph - th, t, th], c);
    push_quad(out, [px, py + ph - t, th, t], c);
    push_quad(out, [px + pw - t, py + ph - th, t, th], c);
    push_quad(out, [px + pw - th, py + ph - t, th, t], c);
    push_quad(
        out,
        [px + t * 0.35, py, (pw - t * 0.7).max(0.0), th * 0.65],
        color::alpha(color::BRASS, 0.4),
    );
    push_quad(
        out,
        [
            px + t * 0.35,
            py + ph - th * 0.65,
            (pw - t * 0.7).max(0.0),
            th * 0.65,
        ],
        color::alpha(color::BRASS, 0.4),
    );
}

/// Career summary formerly drawn as a left frieze over the archive room.
fn push_career_section(
    inner_x: f32,
    inner_top: f32,
    inner_w: f32,
    scroll_y: f32,
    progress: &PlayerProgress,
    title_px: f32,
    body: f32,
    caption_px: f32,
    line_h: f32,
    title_h: f32,
    gap: f32,
    doc_y: &mut f32,
    out_labels: &mut Vec<TextLabel>,
) {
    out_labels.push(TextLabel {
        rect: [inner_x, inner_top + *doc_y - scroll_y, inner_w, title_h],
        text: "CAREER".into(),
        color: color::GOLD,
        font_px: Some(title_px),
        align: TextAlign::Left,
        ..Default::default()
    });
    *doc_y += title_h + gap * 0.85;

    for line in archive_career::career_frieze_lines(progress) {
        out_labels.push(TextLabel {
            rect: [inner_x, inner_top + *doc_y - scroll_y, inner_w, line_h],
            text: line,
            color: color::alpha(color::PARCHMENT, 0.96),
            font_px: Some(body),
            align: TextAlign::Left,
            ..Default::default()
        });
        *doc_y += line_h + gap * 0.45;
    }

    out_labels.push(TextLabel {
        rect: [inner_x, inner_top + *doc_y - scroll_y, inner_w, line_h * 1.05],
        text: archive_career::CHRONICLE_TUTORIAL_NOTE.into(),
        color: color::alpha(color::STONE, 0.95),
        font_px: Some(caption_px),
        align: TextAlign::Left,
        ..Default::default()
    });
    *doc_y += line_h * 1.05 + gap * 1.35;
}

/// Pushes chart quads and labels into `out_quads` / `out_labels`. Does not push the dimming gradient.
pub fn push_chronicle_dashboard(
    w: f32,
    h: f32,
    panel: [f32; 4],
    scroll_y: f32,
    progress: &PlayerProgress,
    out_quads: &mut Vec<GpuInstance>,
    out_labels: &mut Vec<TextLabel>,
) {
    let [px, py, pw, ph] = panel;
    let margin = chronicle_panel_margin(w);
    let inner_x = px + margin;
    let inner_w = (pw - margin * 2.0).max(40.0);
    let inner_top = py + margin;

    let (body, title_px, line_h, title_h, gap, chart_h, bar_row_h) = layout_constants(h);
    let caption_px = typography::size(typography::H42, h);
    let content_h = chronicle_dashboard_content_height(w, h, progress);
    let grid_line = color::alpha(color::UMBER, 0.55);

    let runs = serious_runs_chronological(progress);

    push_quad(
        out_quads,
        [
            inner_x,
            inner_top - scroll_y,
            inner_w,
            content_h.max(ph - margin * 2.0),
        ],
        color::alpha(color::WALNUT_DEEP, 0.96),
    );

    let mut doc_y = gap;
    push_career_section(
        inner_x,
        inner_top,
        inner_w,
        scroll_y,
        progress,
        title_px,
        body,
        caption_px,
        line_h,
        title_h,
        gap,
        &mut doc_y,
        out_labels,
    );

    if runs.is_empty() {
        out_labels.push(TextLabel {
            rect: [inner_x, inner_top + doc_y - scroll_y, inner_w, title_h],
            text: "RUN LOG".into(),
            color: color::GOLD,
            font_px: Some(title_px),
            align: TextAlign::Left,
            ..Default::default()
        });
        doc_y += title_h + gap * 0.85;
        out_labels.push(TextLabel {
            rect: [
                inner_x,
                inner_top + doc_y - scroll_y,
                inner_w,
                line_h * 2.4,
            ],
            text: "Complete a non-tutorial run to populate charts below.\nTutorial runs are excluded from the index."
                .into(),
            color: color::alpha(color::STONE, 0.98),
            font_px: Some(body),
            align: TextAlign::Left,
            ..Default::default()
        });
        push_panel_viewport_frame(out_quads, px, py, pw, ph, w, h);
        return;
    }

    out_labels.push(TextLabel {
        rect: [
            inner_x,
            inner_top + doc_y - scroll_y,
            inner_w,
            line_h * 0.92,
        ],
        text: format!(
            "SERIOUS RUNS · N={} · BUCKET≤{}",
            runs.len(),
            MAX_SCORE_BUCKETS
        ),
        color: color::alpha(color::STONE, 0.92),
        font_px: Some(caption_px),
        align: TextAlign::Left,
        ..Default::default()
    });
    doc_y += line_h * 1.05;

    out_labels.push(TextLabel {
        rect: [inner_x, inner_top + doc_y - scroll_y, inner_w, title_h],
        text: "RUN SCORE — CHRONOLOGICAL PEAK".into(),
        color: color::GOLD,
        font_px: Some(title_px),
        align: TextAlign::Left,
        ..Default::default()
    });
    doc_y += title_h + gap * 0.85;

    let chart_top = inner_top + doc_y - scroll_y;
    let (peaks, max_score) = bucket_peak_scores(&runs);
    let bn = peaks.len().max(1);
    let slot = inner_w / bn as f32;
    let col_w = (slot - 3.0).max(2.0);

    for frac in [0.25_f32, 0.5, 0.75] {
        let gy = chart_top + chart_h * frac;
        push_quad(out_quads, [inner_x, gy, inner_w, 1.0], grid_line);
    }

    for (i, pk) in peaks.iter().enumerate() {
        let x = inner_x + i as f32 * slot + 1.5;
        let frac = *pk as f32 / max_score as f32;
        let bar_h = (chart_h * frac).max(3.0);
        let y0 = chart_top + chart_h - bar_h;
        push_quad(
            out_quads,
            [x, y0, col_w, bar_h],
            color::alpha(color::BRASS, 0.82),
        );
        push_quad(
            out_quads,
            [x, y0, col_w, 1.5],
            color::alpha(color::CHAMPAGNE, 0.5),
        );
    }
    push_quad(
        out_quads,
        [inner_x, chart_top + chart_h, inner_w, 2.0],
        color::alpha(color::GOLD, 0.55),
    );
    doc_y += chart_h + gap * 1.75;

    let mut wins = 0u32;
    let mut losses = 0u32;
    for r in &runs {
        match r.outcome {
            RunOutcome::Victory => wins += 1,
            RunOutcome::Defeat { .. } => losses += 1,
        }
    }
    let total_o = (wins + losses).max(1);

    out_labels.push(TextLabel {
        rect: [inner_x, inner_top + doc_y - scroll_y, inner_w, title_h],
        text: "OUTCOME SPLIT".into(),
        color: color::GOLD,
        font_px: Some(title_px),
        align: TextAlign::Left,
        ..Default::default()
    });
    doc_y += title_h + gap * 0.85;

    let bar_y = inner_top + doc_y - scroll_y;
    let bar_h = bar_row_h * 0.88;
    let inset = inner_w * 0.04;
    let track_x = inner_x + inset;
    let track_w = inner_w - inset * 2.0;
    let w_frac = wins as f32 / total_o as f32;
    let l_frac = losses as f32 / total_o as f32;
    push_quad(
        out_quads,
        [track_x, bar_y + bar_h * 0.5 - 1.0, track_w, 2.0],
        grid_line,
    );
    push_quad(
        out_quads,
        [track_x, bar_y, track_w * w_frac, bar_h],
        color::alpha(color::JADE, 0.88),
    );
    push_quad(
        out_quads,
        [track_x + track_w * w_frac, bar_y, track_w * l_frac, bar_h],
        color::alpha(color::RUBY, 0.88),
    );
    doc_y += bar_h + gap * 0.55;
    out_labels.push(TextLabel {
        rect: [inner_x, inner_top + doc_y - scroll_y, inner_w, line_h],
        text: format!("WIN {wins}  ·  LOSS {losses}  ·  TOTAL {total_o}"),
        color: color::alpha(color::PARCHMENT, 0.94),
        font_px: Some(caption_px),
        align: TextAlign::Left,
        ..Default::default()
    });
    doc_y += line_h + gap * 1.35;

    let mut yaku: Vec<(YakuKind, u32)> = progress
        .yaku_times_scored
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    yaku.sort_by(|a, b| b.1.cmp(&a.1));
    let max_y = yaku.first().map(|(_, c)| *c).unwrap_or(1).max(1);

    out_labels.push(TextLabel {
        rect: [inner_x, inner_top + doc_y - scroll_y, inner_w, title_h],
        text: "YAKU FREQUENCY — CAREER".into(),
        color: color::GOLD,
        font_px: Some(title_px),
        align: TextAlign::Left,
        ..Default::default()
    });
    doc_y += title_h + gap * 0.85;

    let label_w = (inner_w * 0.40).min(240.0);
    let bar_x0 = inner_x + label_w + 10.0;
    let bar_max_w = inner_w - label_w - 20.0;
    for (yk, count) in yaku.into_iter().take(7) {
        let row_top = inner_top + doc_y - scroll_y;
        push_quad(
            out_quads,
            [inner_x, row_top + bar_row_h, inner_w, 1.0],
            color::alpha(color::UMBER, 0.45),
        );
        out_labels.push(TextLabel {
            rect: [inner_x, row_top, label_w, bar_row_h],
            text: yk.name().to_uppercase(),
            color: color::alpha(color::STONE, 0.98),
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        });
        let bw = bar_max_w * (count as f32 / max_y as f32);
        push_quad(
            out_quads,
            [
                bar_x0,
                row_top + bar_row_h * 0.14,
                bw.max(3.0),
                bar_row_h * 0.72,
            ],
            color::alpha(color::LAPIS, 0.72),
        );
        push_quad(
            out_quads,
            [bar_x0, row_top + bar_row_h * 0.14, bw.max(3.0), 1.25],
            color::alpha(color::TALLOW, 0.4),
        );
        out_labels.push(TextLabel {
            rect: [
                bar_x0 + bw + 8.0,
                row_top,
                (inner_w - label_w - bw - 16.0).max(40.0),
                bar_row_h,
            ],
            text: format!("{count}"),
            color: color::alpha(color::PARCHMENT, 0.95),
            font_px: Some(body),
            align: TextAlign::Left,
            ..Default::default()
        });
        doc_y += bar_row_h + 5.0;
    }

    push_panel_viewport_frame(out_quads, px, py, pw, ph, w, h);
}

/// Dimming panel over the scroll band (single gradient quad).
pub fn chronicle_dim_gradient(panel: [f32; 4]) -> GradientQuadInstance {
    GradientQuadInstance {
        rect: panel,
        color: [
            color::WALNUT_INK[0],
            color::WALNUT_INK[1],
            color::WALNUT_INK[2],
            0.82,
        ],
        feather: [0.08, 0.0, 0.0, 0.0],
    }
}
