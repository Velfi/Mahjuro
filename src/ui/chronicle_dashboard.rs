//! Split-pane Chronicle ledger for the Archive tab: run log (left) + career stats (right).
//!
//! Cool twilight sheet over the dimmed archive room; brass rim; walnut-compatible typography.

use crate::core::progression::{PlayerProgress, RunOutcome, RunRecord};
use crate::core::yaku::YakuKind;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, GradientQuadInstance, TextAlign, TextLabel};
use crate::scenes::archive_career;
use crate::ui::tooltip::FRAME_BORDER_PX;

/// Cap score history columns when many runs exist.
const MAX_SCORE_BUCKETS: usize = 48;
/// Below this count, draw one bar per run (no bucketing).
const UNBUCKETED_RUN_MAX: usize = 6;
const LEFT_PANE_FRAC: f32 = 0.42;

/// Scroll offsets and run-list focus for one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChronicleView {
    pub focused_run: Option<usize>,
    pub run_log_scroll: f32,
    pub career_scroll: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ChroniclePaneLayout {
    pub margin: f32,
    pub inner_x: f32,
    pub inner_y: f32,
    pub inner_w: f32,
    pub inner_h: f32,
    pub left_x: f32,
    pub left_w: f32,
    pub right_x: f32,
    pub right_w: f32,
    pub gutter: f32,
    pub run_row_h: f32,
}

pub fn chronicle_pane_layout(w: f32, h: f32, panel: [f32; 4]) -> ChroniclePaneLayout {
    let [px, py, pw, ph] = panel;
    let margin = chronicle_panel_margin(w);
    let inner_x = px + margin;
    let inner_y = py + margin;
    let inner_w = (pw - margin * 2.0).max(80.0);
    let inner_h = (ph - margin * 2.0).max(60.0);
    let gutter = (w * 0.014).max(8.0);
    let left_w = ((inner_w - gutter) * LEFT_PANE_FRAC).max(80.0);
    let right_w = inner_w - gutter - left_w;
    let run_row_h = (typography::size(typography::H36, h) / 0.55).ceil() + 10.0;
    ChroniclePaneLayout {
        margin,
        inner_x,
        inner_y,
        inner_w,
        inner_h,
        left_x: inner_x,
        left_w,
        right_x: inner_x + left_w + gutter,
        right_w,
        gutter,
        run_row_h,
    }
}

#[inline]
pub fn chronicle_panel_margin(w: f32) -> f32 {
    (w * 0.022).max(14.0)
}

fn serious_runs_chronological(progress: &PlayerProgress) -> Vec<&RunRecord> {
    let mut v: Vec<&RunRecord> = progress
        .run_history
        .iter()
        .filter(|r| !r.tutorial_run)
        .collect();
    v.sort_by_key(|r| r.timestamp_unix);
    v
}

fn score_chart_columns(runs: &[&RunRecord]) -> (Vec<u64>, u64) {
    let n = runs.len();
    if n == 0 {
        return (Vec::new(), 1);
    }
    if n <= UNBUCKETED_RUN_MAX {
        let peak: Vec<u64> = runs.iter().map(|r| r.total_score_earned).collect();
        let mx = peak.iter().copied().max().unwrap_or(1).max(1);
        return (peak, mx);
    }
    let b = n.min(MAX_SCORE_BUCKETS);
    let mut peak = vec![0u64; b];
    for i in 0..n {
        let bi = ((i as u64 * b as u64) / n as u64) as usize;
        let bi = bi.min(b - 1);
        peak[bi] = peak[bi].max(runs[i].total_score_earned);
    }
    let mx = peak.iter().copied().max().unwrap_or(1).max(1);
    (peak, mx)
}

fn layout_constants(h: f32) -> (f32, f32, f32, f32, f32, f32, f32) {
    let body = typography::size(typography::H36, h);
    let hero_px = typography::size(typography::H16, h);
    let line_h = (body / 0.55).ceil() + 4.0;
    let section_px = typography::size(typography::H32, h);
    let title_h = (section_px / 0.55).ceil() + 4.0;
    let gap = (h * 0.016).max(10.0);
    let chart_h = (h * 0.11).max(72.0);
    let bar_row_h = (h * 0.030).max(22.0);
    (body, hero_px, line_h, title_h, gap, chart_h, bar_row_h)
}

fn run_log_list_viewport_h(layout: ChroniclePaneLayout, title_h: f32, gap: f32) -> f32 {
    (layout.inner_h - title_h - gap * 0.75).max(1.0)
}

fn run_log_list_content_height(entry_count: usize, layout: ChroniclePaneLayout) -> f32 {
    if entry_count == 0 {
        layout.run_row_h * 2.0
    } else {
        entry_count as f32 * layout.run_row_h
    }
}

fn career_tile_height(h: f32) -> f32 {
    let cap_h = (typography::size(typography::H42, h) / 0.55).ceil();
    let val_h = (typography::size(typography::H36, h) / 0.55).ceil();
    (cap_h * 2.0 + val_h + 18.0).max(h * 0.13).max(96.0)
}

fn career_content_height(
    w: f32,
    h: f32,
    progress: &PlayerProgress,
    _pane_w: f32,
    title_h: f32,
    gap: f32,
    chart_h: f32,
    bar_row_h: f32,
) -> f32 {
    let runs = serious_runs_chronological(progress);
    let scale = metrics::scene_scale(w, h);
    let tile_h = career_tile_height(h);
    let tile_rows = 2.0_f32;
    let mut doc_y = title_h + gap * 0.75;
    doc_y += tile_rows * tile_h + gap * 0.45 + gap * 1.1;
    doc_y += (typography::size(typography::H42, h) / 0.55).ceil() + gap * 0.5;
    if runs.is_empty() {
        doc_y += title_h + chart_h * 0.5;
        return doc_y + (scale * 10.0).max(8.0);
    }
    doc_y += title_h + gap * 0.75 + chart_h + gap * 1.35;
    doc_y += title_h + gap * 0.75 + bar_row_h * 0.88 + gap * 0.5 + (typography::size(typography::H42, h) / 0.55).ceil();
    doc_y += gap + title_h + gap * 0.75;
    let mut yaku: Vec<(YakuKind, u32)> = progress
        .yaku_times_scored
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    yaku.sort_by(|a, b| b.1.cmp(&a.1));
    doc_y += yaku.len().min(5) as f32 * (bar_row_h + 4.0);
    doc_y += (typography::size(typography::H42, h) / 0.55).ceil() * 1.2;
    doc_y + (scale * 10.0).max(8.0)
}

pub fn chronicle_run_log_scroll_max(w: f32, h: f32, panel: [f32; 4], entry_count: usize) -> f32 {
    let panes = chronicle_pane_layout(w, h, panel);
    let (_body, _hero, _line_h, title_h, gap, _ch, _br) = layout_constants(h);
    let list_h = run_log_list_content_height(entry_count, panes);
    let view_h = run_log_list_viewport_h(panes, title_h, gap);
    (list_h - view_h).max(0.0)
}

pub fn chronicle_run_detail_scroll_max(
    w: f32,
    h: f32,
    panel: [f32; 4],
    rec: &RunRecord,
) -> f32 {
    let panes = chronicle_pane_layout(w, h, panel);
    let (_body, _hero, line_h, title_h, gap, _ch, _br) = layout_constants(h);
    let content = run_detail_content_height(rec, title_h, gap, line_h);
    (content - panes.inner_h).max(0.0)
}

/// Max scroll for the right pane given the focused run-log row (`0` = Summary).
pub fn chronicle_right_pane_scroll_max(
    w: f32,
    h: f32,
    panel: [f32; 4],
    progress: &PlayerProgress,
    focused_run: Option<usize>,
) -> f32 {
    if focused_run.is_none() || focused_run == Some(0) {
        chronicle_career_scroll_max(w, h, panel, progress)
    } else if let Some(idx) = focused_run.and_then(|i| {
        archive_career::chronicle_hist_index_at_list(i, progress)
    }) && let Some(rec) = progress.run_history.get(idx)
    {
        chronicle_run_detail_scroll_max(w, h, panel, rec)
    } else {
        0.0
    }
}

pub fn chronicle_career_scroll_max(w: f32, h: f32, panel: [f32; 4], progress: &PlayerProgress) -> f32 {
    let panes = chronicle_pane_layout(w, h, panel);
    let (_body, _hero, _line_h, title_h, gap, chart_h, bar_row_h) = layout_constants(h);
    let content = career_content_height(
        w,
        h,
        progress,
        panes.right_w,
        title_h,
        gap,
        chart_h,
        bar_row_h,
    );
    (content - panes.inner_h).max(0.0)
}

/// Clamp run-log list scroll so `focused_run` stays in the viewport.
pub fn chronicle_clamp_run_log_scroll(
    scroll: f32,
    focused_run: Option<usize>,
    entry_count: usize,
    layout: ChroniclePaneLayout,
) -> f32 {
    let title_h = title_h_for_layout(layout);
    let gap = gap_for_layout(layout);
    let view_h = run_log_list_viewport_h(layout, title_h, gap);
    let max_s = (run_log_list_content_height(entry_count, layout) - view_h).max(0.0);
    let mut scroll = scroll.clamp(0.0, max_s);
    if let Some(i) = focused_run
        && entry_count > 0
    {
        let i = i.min(entry_count - 1);
        let row_top = i as f32 * layout.run_row_h;
        let row_bottom = row_top + layout.run_row_h;
        if row_top < scroll {
            scroll = row_top;
        } else if row_bottom > scroll + view_h {
            scroll = (row_bottom - view_h).max(0.0);
        }
    }
    scroll.clamp(0.0, max_s)
}

#[inline]
fn title_h_for_layout(layout: ChroniclePaneLayout) -> f32 {
    (typography::size(typography::H28, layout.inner_h + layout.margin * 2.0) / 0.55).ceil() + 4.0
}

#[inline]
fn gap_for_layout(layout: ChroniclePaneLayout) -> f32 {
    ((layout.inner_h + layout.margin * 2.0) * 0.016).max(10.0)
}

/// Hit rects for run-log rows (index matches `tab_artifacts` Chronicle order).
pub fn chronicle_run_log_hit_rects(
    w: f32,
    h: f32,
    panel: [f32; 4],
    scroll: f32,
    entry_count: usize,
) -> Vec<[f32; 4]> {
    if entry_count == 0 {
        return Vec::new();
    }
    let panes = chronicle_pane_layout(w, h, panel);
    let (_body, _hero, _line_h, title_h, gap, _ch, _br) = layout_constants(h);
    let list_top = panes.inner_y + title_h + gap * 0.75;
    (0..entry_count)
        .map(|i| {
            let y = list_top + i as f32 * panes.run_row_h - scroll;
            [panes.left_x, y, panes.left_w, panes.run_row_h]
        })
        .filter(|r| r[1] + r[3] > panes.inner_y && r[1] < panes.inner_y + panes.inner_h)
        .collect()
}

#[inline]
fn pane_clip_y(panes: ChroniclePaneLayout) -> (f32, f32) {
    (panes.inner_y, panes.inner_y + panes.inner_h)
}

#[inline]
fn rect_in_clip(y: f32, h: f32, clip_top: f32, clip_bottom: f32) -> bool {
    y + h > clip_top && y < clip_bottom
}

fn push_label_clipped(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    clip_top: f32,
    clip_bottom: f32,
    mut label: TextLabel,
) {
    if rect_in_clip(rect[1], rect[3], clip_top, clip_bottom) {
        let clip_y0 = rect[1].max(clip_top);
        let clip_y1 = (rect[1] + rect[3]).min(clip_bottom);
        if clip_y1 <= clip_y0 {
            return;
        }
        label.clip_rect = Some([rect[0], clip_y0, rect[2].max(0.0), clip_y1 - clip_y0]);
        out.push(label);
    }
}

fn push_quad_clipped(
    out: &mut Vec<GpuInstance>,
    rect: [f32; 4],
    clip_top: f32,
    clip_bottom: f32,
    color: [f32; 4],
) {
    if rect_in_clip(rect[1], rect[3], clip_top, clip_bottom) {
        push_quad(out, rect, color);
    }
}

fn run_detail_content_height(rec: &RunRecord, title_h: f32, gap: f32, line_h: f32) -> f32 {
    let desc_lines = archive_career::chronicle_run_description(rec)
        .lines()
        .count()
        .max(1) as f32;
    let stat_lines = archive_career::chronicle_run_stats(rec)
        .lines()
        .filter(|l| !l.is_empty())
        .count()
        .max(1) as f32;
    title_h + gap * 0.75 + title_h + gap * 0.5 + desc_lines * line_h + gap + title_h + gap * 0.5
        + stat_lines * line_h
        + gap
}

fn push_quad(out: &mut Vec<GpuInstance>, rect: [f32; 4], color: [f32; 4]) {
    out.push(GpuInstance {
        rect,
        color,
        user: 0,
    });
}

fn push_ledger_sheet(
    out: &mut Vec<GpuInstance>,
    inner_x: f32,
    inner_y: f32,
    inner_w: f32,
    inner_h: f32,
) {
    let b = FRAME_BORDER_PX;
    push_quad(
        out,
        [inner_x - b, inner_y - b, inner_w + b * 2.0, inner_h + b * 2.0],
        color::alpha(color::BRASS, 0.5),
    );
    push_quad(
        out,
        [inner_x, inner_y, inner_w, inner_h],
        [
            color::TWILIGHT[0],
            color::TWILIGHT[1],
            color::TWILIGHT[2],
            0.94,
        ],
    );
}

fn push_ruled_lines(
    out: &mut Vec<GpuInstance>,
    x: f32,
    y0: f32,
    w: f32,
    h: f32,
    scroll: f32,
    spacing: f32,
) {
    let line_c = color::alpha(color::UMBER, 0.32);
    let mut y = y0 - scroll % spacing;
    while y < y0 + h {
        if y + 1.0 > y0 {
            push_quad(out, [x, y, w, 1.0], line_c);
        }
        y += spacing;
    }
}

fn push_run_log(
    progress: &PlayerProgress,
    panes: ChroniclePaneLayout,
    scroll: f32,
    focused: Option<usize>,
    section_px: f32,
    body: f32,
    caption_px: f32,
    title_h: f32,
    gap: f32,
    out_quads: &mut Vec<GpuInstance>,
    out_labels: &mut Vec<TextLabel>,
) {
    let (clip_top, clip_bottom) = pane_clip_y(panes);
    let indices = archive_career::chronicle_indices_recent_first(progress);
    let run_count = indices.len();
    let entry_count = archive_career::chronicle_list_entry_count(progress);

    push_label_clipped(
        out_labels,
        [panes.left_x, panes.inner_y, panes.left_w, title_h],
        clip_top,
        clip_bottom,
        TextLabel {
            rect: [panes.left_x, panes.inner_y, panes.left_w, title_h],
            text: "Run log".into(),
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );

    let list_top = panes.inner_y + title_h + gap * 0.75;
    push_ruled_lines(
        out_quads,
        panes.left_x,
        list_top,
        panes.left_w,
        panes.inner_h - title_h - gap * 0.75,
        scroll,
        panes.run_row_h,
    );

    if run_count == 0 {
        let row_y = list_top - scroll;
        push_label_clipped(
            out_labels,
            [panes.left_x, row_y, panes.left_w, panes.run_row_h * 2.0],
            clip_top,
            clip_bottom,
            TextLabel {
                rect: [panes.left_x, row_y, panes.left_w, panes.run_row_h * 2.0],
                text: "Finish a non-tutorial run to add entries here.".into(),
                color: color::alpha(color::STONE, 0.95),
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        return;
    }

    let summary_sub = format!("{run_count} runs · career overview");

    for list_i in 0..entry_count {
        let row_y = list_top + list_i as f32 * panes.run_row_h - scroll;
        if !rect_in_clip(row_y, panes.run_row_h, clip_top, clip_bottom) {
            continue;
        }
        let selected = focused == Some(list_i);
        if selected {
            push_quad_clipped(
                out_quads,
                [panes.left_x, row_y, panes.left_w, panes.run_row_h],
                clip_top,
                clip_bottom,
                color::alpha(color::WALNUT_SOFT, 0.88),
            );
            push_quad_clipped(
                out_quads,
                [panes.left_x, row_y, 3.0, panes.run_row_h],
                clip_top,
                clip_bottom,
                color::alpha(color::CHAMPAGNE, 0.75),
            );
        }

        let (title, subtitle) = if list_i == 0 {
            (
                "Summary".into(),
                summary_sub.clone(),
            )
        } else {
            let hist_idx = indices[list_i - 1];
            let Some(rec) = progress.run_history.get(hist_idx) else {
                continue;
            };
            let display = archive_career::chronicle_display_run_number(list_i, progress)
                .unwrap_or(0);
            (
                archive_career::chronicle_run_log_title(progress, display, rec),
                archive_career::chronicle_run_log_subtitle(rec),
            )
        };

        push_label_clipped(
            out_labels,
            [panes.left_x + 8.0, row_y + 2.0, panes.left_w - 12.0, panes.run_row_h * 0.52],
            clip_top,
            clip_bottom,
            TextLabel {
                rect: [panes.left_x + 8.0, row_y + 2.0, panes.left_w - 12.0, panes.run_row_h * 0.52],
                text: title,
                color: if selected {
                    color::CHAMPAGNE
                } else {
                    color::alpha(color::PARCHMENT, 0.96)
                },
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        push_label_clipped(
            out_labels,
            [
                panes.left_x + 8.0,
                row_y + panes.run_row_h * 0.48,
                panes.left_w - 12.0,
                panes.run_row_h * 0.48,
            ],
            clip_top,
            clip_bottom,
            TextLabel {
                rect: [
                    panes.left_x + 8.0,
                    row_y + panes.run_row_h * 0.48,
                    panes.left_w - 12.0,
                    panes.run_row_h * 0.48,
                ],
                text: subtitle,
                color: color::alpha(color::STONE, 0.92),
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
    }
}

fn push_run_detail_pane(
    progress: &PlayerProgress,
    list_index: usize,
    panes: ChroniclePaneLayout,
    scroll: f32,
    section_px: f32,
    body: f32,
    caption_px: f32,
    title_h: f32,
    gap: f32,
    line_h: f32,
    out_labels: &mut Vec<TextLabel>,
) {
    let Some(hist_idx) = archive_career::chronicle_hist_index_at_list(list_index, progress) else {
        return;
    };
    let Some(rec) = progress.run_history.get(hist_idx) else {
        return;
    };
    let (clip_top, clip_bottom) = pane_clip_y(panes);
    let display = archive_career::chronicle_display_run_number(list_index, progress)
        .unwrap_or(0);
    let mut doc_y = 0.0_f32;

    let heading = archive_career::chronicle_run_log_title(progress, display, rec);
    push_label_clipped(
        out_labels,
        [
            panes.right_x,
            panes.inner_y + doc_y - scroll,
            panes.right_w,
            title_h,
        ],
        clip_top,
        clip_bottom,
        TextLabel {
            rect: [
                panes.right_x,
                panes.inner_y + doc_y - scroll,
                panes.right_w,
                title_h,
            ],
            text: heading,
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    doc_y += title_h + gap * 0.75;

    push_label_clipped(
        out_labels,
        [
            panes.right_x,
            panes.inner_y + doc_y - scroll,
            panes.right_w,
            title_h,
        ],
        clip_top,
        clip_bottom,
        TextLabel {
            rect: [
                panes.right_x,
                panes.inner_y + doc_y - scroll,
                panes.right_w,
                title_h,
            ],
            text: "Run record".into(),
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    doc_y += title_h + gap * 0.5;

    for line in archive_career::chronicle_run_description(rec).lines() {
        push_label_clipped(
            out_labels,
            [
                panes.right_x,
                panes.inner_y + doc_y - scroll,
                panes.right_w,
                line_h,
            ],
            clip_top,
            clip_bottom,
            TextLabel {
                rect: [
                    panes.right_x,
                    panes.inner_y + doc_y - scroll,
                    panes.right_w,
                    line_h,
                ],
                text: line.into(),
                color: color::alpha(color::PARCHMENT, 0.96),
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += line_h;
    }
    doc_y += gap;

    push_label_clipped(
        out_labels,
        [
            panes.right_x,
            panes.inner_y + doc_y - scroll,
            panes.right_w,
            title_h,
        ],
        clip_top,
        clip_bottom,
        TextLabel {
            rect: [
                panes.right_x,
                panes.inner_y + doc_y - scroll,
                panes.right_w,
                title_h,
            ],
            text: "Stats".into(),
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    doc_y += title_h + gap * 0.5;

    for line in archive_career::chronicle_run_stats(rec).lines().filter(|l| !l.is_empty()) {
        push_label_clipped(
            out_labels,
            [
                panes.right_x,
                panes.inner_y + doc_y - scroll,
                panes.right_w,
                line_h,
            ],
            clip_top,
            clip_bottom,
            TextLabel {
                rect: [
                    panes.right_x,
                    panes.inner_y + doc_y - scroll,
                    panes.right_w,
                    line_h,
                ],
                text: line.into(),
                color: color::alpha(color::STONE, 0.94),
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += line_h;
    }
}

fn push_career_pane(
    _w: f32,
    h: f32,
    progress: &PlayerProgress,
    panes: ChroniclePaneLayout,
    scroll: f32,
    section_px: f32,
    body: f32,
    caption_px: f32,
    _hero_px: f32,
    title_h: f32,
    gap: f32,
    chart_h: f32,
    bar_row_h: f32,
    out_quads: &mut Vec<GpuInstance>,
    out_labels: &mut Vec<TextLabel>,
) {
    let runs = serious_runs_chronological(progress);
    let grid_line = color::alpha(color::UMBER, 0.45);
    let (clip_top, clip_bottom) = pane_clip_y(panes);
    let cap_h = (caption_px / 0.55).ceil();
    let val_h = (body / 0.55).ceil();
    let mut doc_y = title_h + gap * 0.75;

    let header_rect = [
        panes.right_x,
        panes.inner_y + doc_y - scroll,
        panes.right_w,
        title_h,
    ];
    push_label_clipped(
        out_labels,
        header_rect,
        clip_top,
        clip_bottom,
        TextLabel {
            rect: header_rect,
            text: "Career".into(),
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    doc_y += title_h + gap * 0.75;

    let tiles = archive_career::career_tiles(progress);
    let tile_gap = gap * 0.45;
    let tile_w = (panes.right_w - tile_gap) * 0.5;
    let tile_h = career_tile_height(h);
    for (i, tile) in tiles.iter().take(4).enumerate() {
        let col = (i % 2) as f32;
        let row = (i / 2) as f32;
        let tx = panes.right_x + col * (tile_w + tile_gap);
        let ty = panes.inner_y + doc_y - scroll + row * (tile_h + tile_gap);
        push_quad_clipped(
            out_quads,
            [tx, ty, tile_w, tile_h],
            clip_top,
            clip_bottom,
            color::alpha(color::WALNUT_INK, 0.55),
        );
        push_quad_clipped(
            out_quads,
            [tx, ty, tile_w, 1.0],
            clip_top,
            clip_bottom,
            color::alpha(color::BRASS, 0.35),
        );
        let is_hero = i == 0 && !runs.is_empty();
        let inner_x = tx + 10.0;
        let inner_w = tile_w - 16.0;
        let label_y = ty + 6.0;
        let value_y = label_y + cap_h + 2.0;
        let detail_y = (ty + tile_h - cap_h - 6.0).max(value_y + val_h + 2.0);
        push_label_clipped(
            out_labels,
            [inner_x, label_y, inner_w, cap_h],
            clip_top,
            clip_bottom,
            TextLabel {
                rect: [inner_x, label_y, inner_w, cap_h],
                text: tile.label.into(),
                color: color::STONE,
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        push_label_clipped(
            out_labels,
            [inner_x, value_y, inner_w, val_h],
            clip_top,
            clip_bottom,
            TextLabel {
                rect: [inner_x, value_y, inner_w, val_h],
                text: tile.value.clone(),
                color: if is_hero {
                    color::CHAMPAGNE
                } else {
                    color::PARCHMENT
                },
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        if let Some(ref detail) = tile.detail {
            push_label_clipped(
                out_labels,
                [inner_x, detail_y, inner_w, cap_h],
                clip_top,
                clip_bottom,
                TextLabel {
                    rect: [inner_x, detail_y, inner_w, cap_h],
                    text: detail.clone(),
                    color: color::alpha(color::STONE, 0.9),
                    font_px: Some(caption_px),
                    align: TextAlign::Left,
                    ..Default::default()
                },
            );
        }
    }
    doc_y += tile_h * 2.0 + tile_gap + gap * 0.85;

    if !runs.is_empty() {
        let n = runs.len();
        let count_rect = [
            panes.right_x,
            panes.inner_y + doc_y - scroll,
            panes.right_w,
            cap_h,
        ];
        push_label_clipped(
            out_labels,
            count_rect,
            clip_top,
            clip_bottom,
            TextLabel {
                rect: count_rect,
                text: format!("{n} runs recorded"),
                color: color::alpha(color::STONE, 0.9),
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += cap_h + gap * 0.5;

        let score_title = [
            panes.right_x,
            panes.inner_y + doc_y - scroll,
            panes.right_w,
            title_h,
        ];
        push_label_clipped(
            out_labels,
            score_title,
            clip_top,
            clip_bottom,
            TextLabel {
                rect: score_title,
                text: "Score history".into(),
                color: color::GOLD,
                font_px: Some(section_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += title_h + gap * 0.65;

        let chart_top = panes.inner_y + doc_y - scroll;
        let (peaks, max_score) = score_chart_columns(&runs);
        let bn = peaks.len().max(1);
        let slot = panes.right_w / bn as f32;
        let col_w = (slot - 4.0).max(3.0).min(slot * 0.85);

        for frac in [0.25_f32, 0.5, 0.75] {
            push_quad_clipped(
                out_quads,
                [panes.right_x, chart_top + chart_h * frac, panes.right_w, 1.0],
                clip_top,
                clip_bottom,
                grid_line,
            );
        }
        for (i, pk) in peaks.iter().enumerate() {
            let x = panes.right_x + i as f32 * slot + (slot - col_w) * 0.5;
            let frac = *pk as f32 / max_score as f32;
            let bar_h = (chart_h * frac).max(3.0);
            let y0 = chart_top + chart_h - bar_h;
            push_quad_clipped(
                out_quads,
                [x, y0, col_w, bar_h],
                clip_top,
                clip_bottom,
                color::alpha(color::CHAMPAGNE, 0.78),
            );
            push_quad_clipped(
                out_quads,
                [x, y0, col_w, 1.25],
                clip_top,
                clip_bottom,
                color::alpha(color::TALLOW, 0.45),
            );
        }
        push_quad_clipped(
            out_quads,
            [panes.right_x, chart_top + chart_h, panes.right_w, 1.5],
            clip_top,
            clip_bottom,
            color::alpha(color::ANTIQUE, 0.7),
        );
        let earlier_rect = [
            panes.right_x,
            chart_top + chart_h + 4.0,
            panes.right_w * 0.5,
            cap_h,
        ];
        push_label_clipped(
            out_labels,
            earlier_rect,
            clip_top,
            clip_bottom,
            TextLabel {
                rect: earlier_rect,
                text: "earlier".into(),
                color: color::alpha(color::STONE, 0.75),
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        let later_rect = [
            panes.right_x + panes.right_w * 0.5,
            chart_top + chart_h + 4.0,
            panes.right_w * 0.5,
            cap_h,
        ];
        push_label_clipped(
            out_labels,
            later_rect,
            clip_top,
            clip_bottom,
            TextLabel {
                rect: later_rect,
                text: "later".into(),
                color: color::alpha(color::STONE, 0.75),
                font_px: Some(caption_px),
                align: TextAlign::Right,
                ..Default::default()
            },
        );
        doc_y += chart_h + gap * 1.25;

        let mut wins = 0u32;
        let mut losses = 0u32;
        for r in &runs {
            match r.outcome {
                RunOutcome::Victory => wins += 1,
                RunOutcome::Defeat { .. } => losses += 1,
            }
        }
        let total_o = (wins + losses).max(1);
        let w_pct = (wins as f32 / total_o as f32 * 100.0).round() as u32;
        let l_pct = (losses as f32 / total_o as f32 * 100.0).round() as u32;

        let outcomes_title = [
            panes.right_x,
            panes.inner_y + doc_y - scroll,
            panes.right_w,
            title_h,
        ];
        push_label_clipped(
            out_labels,
            outcomes_title,
            clip_top,
            clip_bottom,
            TextLabel {
                rect: outcomes_title,
                text: "Outcomes".into(),
                color: color::GOLD,
                font_px: Some(section_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += title_h + gap * 0.65;

        let bar_y = panes.inner_y + doc_y - scroll;
        let bar_h = bar_row_h * 0.88;
        let track_x = panes.right_x;
        let track_w = panes.right_w;
        let w_frac = wins as f32 / total_o as f32;
        push_quad_clipped(
            out_quads,
            [track_x, bar_y + bar_h * 0.5 - 1.0, track_w, 1.5],
            clip_top,
            clip_bottom,
            grid_line,
        );
        push_quad_clipped(
            out_quads,
            [track_x, bar_y, track_w * w_frac, bar_h],
            clip_top,
            clip_bottom,
            color::alpha(color::JADE, 0.85),
        );
        push_quad_clipped(
            out_quads,
            [track_x + track_w * w_frac, bar_y, track_w * (1.0 - w_frac), bar_h],
            clip_top,
            clip_bottom,
            color::alpha(color::RUBY, 0.85),
        );
        doc_y += bar_h + gap * 0.45;
        let pct_rect = [
            panes.right_x,
            panes.inner_y + doc_y - scroll,
            panes.right_w,
            cap_h,
        ];
        push_label_clipped(
            out_labels,
            pct_rect,
            clip_top,
            clip_bottom,
            TextLabel {
                rect: pct_rect,
                text: format!("Win {w_pct}%  ·  Loss {l_pct}%"),
                color: color::alpha(color::PARCHMENT, 0.94),
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += cap_h + gap;

        let mut yaku: Vec<(YakuKind, u32)> = progress
            .yaku_times_scored
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        yaku.sort_by(|a, b| b.1.cmp(&a.1));
        let max_y = yaku.first().map(|(_, c)| *c).unwrap_or(1).max(1);

        let yaku_title = [
            panes.right_x,
            panes.inner_y + doc_y - scroll,
            panes.right_w,
            title_h,
        ];
        push_label_clipped(
            out_labels,
            yaku_title,
            clip_top,
            clip_bottom,
            TextLabel {
                rect: yaku_title,
                text: "Top yaku".into(),
                color: color::GOLD,
                font_px: Some(section_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += title_h + gap * 0.65;

        let label_w = (panes.right_w * 0.42).min(200.0);
        let bar_x0 = panes.right_x + label_w + 8.0;
        let bar_max_w = panes.right_w - label_w - 12.0;
        for (yk, count) in yaku.into_iter().take(5) {
            let row_top = panes.inner_y + doc_y - scroll;
            let name_rect = [panes.right_x, row_top, label_w, bar_row_h];
            push_label_clipped(
                out_labels,
                name_rect,
                clip_top,
                clip_bottom,
                TextLabel {
                    rect: name_rect,
                    text: yk.name().into(),
                    color: color::alpha(color::STONE, 0.96),
                    font_px: Some(caption_px),
                    align: TextAlign::Left,
                    ..Default::default()
                },
            );
            let bw = bar_max_w * (count as f32 / max_y as f32);
            push_quad_clipped(
                out_quads,
                [
                    bar_x0,
                    row_top + bar_row_h * 0.18,
                    bw.max(4.0),
                    bar_row_h * 0.64,
                ],
                clip_top,
                clip_bottom,
                color::alpha(color::FELT_LIT, 0.75),
            );
            let count_rect = [
                bar_x0 + bw + 6.0,
                row_top,
                (panes.right_w - label_w - bw - 10.0).max(28.0),
                bar_row_h,
            ];
            push_label_clipped(
                out_labels,
                count_rect,
                clip_top,
                clip_bottom,
                TextLabel {
                    rect: count_rect,
                    text: format!("{count}"),
                    color: color::alpha(color::PARCHMENT, 0.94),
                    font_px: Some(body),
                    align: TextAlign::Left,
                    ..Default::default()
                },
            );
            doc_y += bar_row_h + 4.0;
        }
    }
}

/// Pushes the split ledger into `out_quads` / `out_labels`. Does not push the room dim gradient.
pub fn push_chronicle_dashboard(
    w: f32,
    h: f32,
    panel: [f32; 4],
    view: ChronicleView,
    progress: &PlayerProgress,
    out_quads: &mut Vec<GpuInstance>,
    out_labels: &mut Vec<TextLabel>,
) {
    let panes = chronicle_pane_layout(w, h, panel);
    let (body, hero_px, line_h, title_h, gap, chart_h, bar_row_h) = layout_constants(h);
    let section_px = typography::size(typography::H28, h);
    let caption_px = typography::size(typography::H42, h);

    push_ledger_sheet(
        out_quads,
        panes.inner_x,
        panes.inner_y,
        panes.inner_w,
        panes.inner_h,
    );

    push_quad(
        out_quads,
        [panes.left_x + panes.left_w, panes.inner_y, panes.gutter, panes.inner_h],
        color::alpha(color::UMBER, 0.35),
    );

    push_run_log(
        progress,
        panes,
        view.run_log_scroll,
        view.focused_run,
        section_px,
        body,
        caption_px,
        title_h,
        gap,
        out_quads,
        out_labels,
    );

    let show_summary = view.focused_run.is_none() || view.focused_run == Some(0);
    if show_summary {
        push_career_pane(
            w,
            h,
            progress,
            panes,
            view.career_scroll,
            section_px,
            body,
            caption_px,
            hero_px,
            title_h,
            gap,
            chart_h,
            bar_row_h,
            out_quads,
            out_labels,
        );
    } else if let Some(list_i) = view.focused_run {
        push_run_detail_pane(
            progress,
            list_i,
            panes,
            view.career_scroll,
            section_px,
            body,
            caption_px,
            title_h,
            gap,
            line_h,
            out_labels,
        );
    }
}

/// Heavier twilight vignette over the archive room.
pub fn chronicle_dim_gradient(panel: [f32; 4]) -> GradientQuadInstance {
    GradientQuadInstance {
        rect: panel,
        color: [
            color::TWILIGHT_INK[0],
            color::TWILIGHT_INK[1],
            color::TWILIGHT_INK[2],
            0.88,
        ],
        feather: [0.12, 0.0, 0.0, 0.0],
    }
}
