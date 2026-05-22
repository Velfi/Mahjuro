//! Split-pane Chronicle ledger for the Archive tab: run log (left) + career stats (right).

use crate::core::progression::{PlayerProgress, RunOutcome, RunRecord};
use crate::core::yaku::YakuKind;
use crate::render::draw_cmd::{ImageQuad, ImageQuadSource};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, GradientQuadInstance, TextAlign, TextLabel};
use crate::scenes::archive_career;
use crate::ui::chart_primitives::{
    self, ChartClip, push_hbar_row, push_quad_clipped as chart_quad, push_sparkline,
    push_stacked_bar, push_vbar_chart,
};
use crate::ui::clip::intersect_rect;
use crate::ui::tooltip::FRAME_BORDER_PX;

const MAX_SCORE_BUCKETS: usize = 48;
const UNBUCKETED_RUN_MAX: usize = 6;
const LEFT_PANE_FRAC: f32 = 0.42;
const KPI_COUNT: usize = 5;

#[derive(Clone, Copy, Debug, Default)]
pub struct ChronicleView {
    pub focused_run: Option<usize>,
    pub focused_pane: ChronicleScrollPane,
    pub run_log_scroll: f32,
    pub career_scroll: f32,
}

/// Which Chronicle ledger column should receive wheel / page scroll.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChronicleScrollPane {
    #[default]
    RunLog,
    Career,
}

/// Screen rects for run log (left) and career/detail (right) columns.
pub fn chronicle_pane_rects(w: f32, h: f32, panel: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let panes = chronicle_pane_layout(w, h, panel);
    let band = [panes.content_y(), panes.content_h()];
    (
        [panes.left_content_x(), band[0], panes.left_content_w(), band[1]],
        [panes.right_content_x(), band[0], panes.right_content_w(), band[1]],
    )
}

/// Pane under the cursor (run log vs career/detail). `None` when outside the ledger.
pub fn chronicle_scroll_pane_at(
    w: f32,
    h: f32,
    panel: [f32; 4],
    cursor: (f32, f32),
) -> Option<ChronicleScrollPane> {
    let panes = chronicle_pane_layout(w, h, panel);
    let (cx, cy) = cursor;
    if cy < panes.content_y() || cy > panes.content_y() + panes.content_h() {
        return None;
    }
    if cx >= panes.left_content_x() && cx < panes.left_content_x() + panes.left_content_w() {
        Some(ChronicleScrollPane::RunLog)
    } else if cx >= panes.right_content_x() && cx < panes.right_content_x() + panes.right_content_w() {
        Some(ChronicleScrollPane::Career)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ChroniclePaneLayout {
    pub margin: f32,
    pub inner_x: f32,
    pub inner_y: f32,
    pub inner_w: f32,
    pub inner_h: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    pub left_x: f32,
    pub left_w: f32,
    pub right_x: f32,
    pub right_w: f32,
    pub gutter: f32,
    pub run_row_h: f32,
}

impl ChroniclePaneLayout {
    #[inline]
    pub fn content_y(&self) -> f32 {
        self.inner_y + self.pad_y
    }

    #[inline]
    pub fn content_h(&self) -> f32 {
        (self.inner_h - self.pad_y * 2.0).max(1.0)
    }

    #[inline]
    pub fn left_content_x(&self) -> f32 {
        self.left_x + self.pad_x
    }

    #[inline]
    pub fn left_content_w(&self) -> f32 {
        (self.left_w - self.pad_x * 2.0).max(40.0)
    }

    #[inline]
    pub fn right_content_x(&self) -> f32 {
        self.right_x + self.pad_x
    }

    #[inline]
    pub fn right_content_w(&self) -> f32 {
        (self.right_w - self.pad_x * 2.0).max(40.0)
    }
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
    let body_line = (typography::size(typography::H36, h) / 0.55).ceil() + 2.0;
    let cap_line = (typography::size(typography::H42, h) / 0.55).ceil() + 2.0;
    let run_row_h = body_line + cap_line + cap_line + 10.0;
    let pad_x = (w * 0.018).max(12.0);
    let pad_y = (h * 0.020).max(14.0);
    ChroniclePaneLayout {
        margin,
        inner_x,
        inner_y,
        inner_w,
        inner_h,
        pad_x,
        pad_y,
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

/// Panel height for chronicle layout/scroll (uses the full archive content band).
pub fn chronicle_panel_height(_w: f32, _h: f32, band_h: f32) -> f32 {
    band_h.max(120.0)
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

pub fn score_chart_columns(runs: &[&RunRecord]) -> (Vec<u64>, u64) {
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
    for (i, run) in runs.iter().enumerate().take(n) {
        let bi = ((i as u64 * b as u64) / n as u64) as usize;
        let bi = bi.min(b - 1);
        peak[bi] = peak[bi].max(run.total_score_earned);
    }
    let mx = peak.iter().copied().max().unwrap_or(1).max(1);
    (peak, mx)
}

#[derive(Clone, Copy, Debug)]
struct ChronicleLayoutMetrics {
    body: f32,
    line_h: f32,
    title_h: f32,
    gap: f32,
    chart_h: f32,
    bar_row_h: f32,
    kpi_h: f32,
    hero_h: f32,
    footer_h: f32,
}

fn layout_constants(h: f32) -> ChronicleLayoutMetrics {
    let body = typography::size(typography::H36, h);
    let line_h = (body / 0.55).ceil() + 4.0;
    let section_px = typography::size(typography::H32, h);
    let title_h = (section_px / 0.55).ceil() + 4.0;
    let gap = (h * 0.016).max(10.0);
    let chart_h = (h * 0.10).max(68.0);
    let bar_row_h = (h * 0.028).max(20.0);
    let kpi_h = (h * 0.095).max(64.0);
    let hero_h = (h * 0.14).max(100.0);
    let footer_h = (typography::size(typography::H42, h) / 0.55).ceil() + 14.0;
    ChronicleLayoutMetrics {
        body,
        line_h,
        title_h,
        gap,
        chart_h,
        bar_row_h,
        kpi_h,
        hero_h,
        footer_h,
    }
}

#[derive(Clone, Copy, Debug)]
struct ChronicleTypeScale {
    section_px: f32,
    body: f32,
    caption_px: f32,
    metrics: ChronicleLayoutMetrics,
}

struct ChronicleEmit<'a> {
    quads: &'a mut Vec<GpuInstance>,
    labels: &'a mut Vec<TextLabel>,
    images: &'a mut Vec<ImageQuad>,
}

struct ChroniclePaneDraw<'a> {
    progress: &'a PlayerProgress,
    panes: ChroniclePaneLayout,
    scroll: f32,
    type_scale: ChronicleTypeScale,
    emit: ChronicleEmit<'a>,
}

fn run_log_list_viewport_h(layout: ChroniclePaneLayout, title_h: f32, gap: f32) -> f32 {
    (layout.content_h() - title_h - gap * 0.85).max(1.0)
}

struct RunLogListLayout {
    list_top: f32,
    header_band_h: f32,
    summary_row_h: f32,
}

fn run_log_list_layout(panes: ChroniclePaneLayout, title_h: f32, gap: f32, cap_h: f32) -> RunLogListLayout {
    RunLogListLayout {
        list_top: panes.content_y() + title_h + gap * 0.75,
        header_band_h: cap_h + 6.0,
        summary_row_h: cap_h + 6.0,
    }
}

fn run_log_row_y(layout: &RunLogListLayout, list_i: usize, run_row_h: f32, scroll: f32) -> (f32, f32) {
    if list_i == 0 {
        (
            layout.list_top + layout.header_band_h - scroll,
            layout.summary_row_h,
        )
    } else {
        let runs_top = layout.list_top + layout.header_band_h + layout.summary_row_h;
        (
            runs_top + (list_i - 1) as f32 * run_row_h - scroll,
            run_row_h,
        )
    }
}

fn run_log_list_content_height(
    entry_count: usize,
    layout: &RunLogListLayout,
    run_row_h: f32,
) -> f32 {
    if entry_count == 0 {
        return layout.header_band_h + run_row_h * 2.0;
    }
    layout.header_band_h
        + layout.summary_row_h
        + (entry_count.saturating_sub(1)) as f32 * run_row_h
}

fn highlight_tile_height(h: f32) -> f32 {
    let cap_h = (typography::size(typography::H42, h) / 0.55).ceil();
    let val_h = (typography::size(typography::H36, h) / 0.55).ceil();
    (cap_h + val_h + cap_h + 24.0).max(h * 0.09).max(72.0)
}

fn career_content_height(w: f32, h: f32, progress: &PlayerProgress, m: ChronicleLayoutMetrics) -> f32 {
    let runs = serious_runs_chronological(progress);
    let scale = metrics::scene_scale(w, h);
    let cap = (typography::size(typography::H42, h) / 0.55).ceil();
    let mut doc_y = m.title_h + m.gap;
    doc_y += m.kpi_h + m.gap;
    doc_y += (h * 0.16).max(110.0) + m.gap;
    let highlight_rows = ((archive_career::career_tiles(progress).len().min(4) + 1) / 2).max(1) as f32;
    doc_y += highlight_tile_height(h) * highlight_rows + m.gap;
    if runs.is_empty() {
        return doc_y + m.footer_h + scale * 8.0;
    }
    doc_y += cap + m.gap * 0.5;
    doc_y += m.title_h + m.chart_h + cap + m.gap;
    doc_y += m.title_h + m.bar_row_h + cap + m.gap;
    doc_y += m.title_h + m.gap;
    let yaku_n = progress.yaku_times_scored.len().min(6);
    doc_y += yaku_n as f32 * (m.bar_row_h + 4.0);
    doc_y += m.footer_h + m.gap;
    doc_y + scale * 10.0
}

fn run_detail_content_height(
    model: &archive_career::RunDetailModel,
    m: ChronicleLayoutMetrics,
) -> f32 {
    let mut h = m.title_h * 2.0 + m.gap * 2.0;
    h += m.hero_h + m.gap;
    h += model.yaku_rows.len().min(8) as f32 * m.bar_row_h + m.gap;
    h += m.title_h + m.chart_h + m.gap;
    if !model.timeline.is_empty() {
        h += m.title_h + model.timeline.len() as f32 * m.line_h * 0.9 + m.gap;
    }
    h += m.title_h + m.line_h + m.gap;
    h += m.footer_h + m.gap * 2.0;
    h
}

pub fn chronicle_run_log_scroll_max(w: f32, h: f32, panel: [f32; 4], entry_count: usize) -> f32 {
    let panes = chronicle_pane_layout(w, h, panel);
    let m = layout_constants(h);
    let cap_h = (typography::size(typography::H42, h) / 0.55).ceil();
    let list_layout = run_log_list_layout(panes, m.title_h, m.gap, cap_h);
    let list_h = run_log_list_content_height(entry_count, &list_layout, panes.run_row_h);
    let view_h = run_log_list_viewport_h(panes, m.title_h, m.gap);
    (list_h - view_h).max(0.0)
}

pub fn chronicle_run_detail_scroll_max(
    w: f32,
    h: f32,
    panel: [f32; 4],
    progress: &PlayerProgress,
    list_index: usize,
) -> f32 {
    let panes = chronicle_pane_layout(w, h, panel);
    let m = layout_constants(h);
    let Some(hist_idx) = archive_career::chronicle_hist_index_at_list(list_index, progress) else {
        return 0.0;
    };
    let Some(rec) = progress.run_history.get(hist_idx) else {
        return 0.0;
    };
    let display = archive_career::chronicle_display_run_number(list_index, progress).unwrap_or(0);
    let model = archive_career::run_detail_model(progress, display, rec);
    let content = run_detail_content_height(&model, m);
    (content - panes.content_h()).max(0.0)
}

pub fn chronicle_right_pane_scroll_max(
    w: f32,
    h: f32,
    panel: [f32; 4],
    progress: &PlayerProgress,
    focused_run: Option<usize>,
) -> f32 {
    if focused_run.is_none() || focused_run == Some(0) {
        chronicle_career_scroll_max(w, h, panel, progress)
    } else if let Some(list_i) = focused_run {
        chronicle_run_detail_scroll_max(w, h, panel, progress, list_i)
    } else {
        0.0
    }
}

pub fn chronicle_career_scroll_max(
    w: f32,
    h: f32,
    panel: [f32; 4],
    progress: &PlayerProgress,
) -> f32 {
    let panes = chronicle_pane_layout(w, h, panel);
    let m = layout_constants(h);
    let content = career_content_height(w, h, progress, m);
    (content - panes.content_h()).max(0.0)
}

pub fn chronicle_clamp_run_log_scroll(
    scroll: f32,
    focused_run: Option<usize>,
    entry_count: usize,
    layout: ChroniclePaneLayout,
    follow_focus: bool,
) -> f32 {
    let title_h = title_h_for_layout(layout);
    let gap = gap_for_layout(layout);
    let cap_h = (typography::size(typography::H42, layout.inner_h + layout.margin * 2.0) / 0.55)
        .ceil();
    let list_layout = run_log_list_layout(layout, title_h, gap, cap_h);
    let view_h = run_log_list_viewport_h(layout, title_h, gap);
    let max_s = (run_log_list_content_height(entry_count, &list_layout, layout.run_row_h) - view_h)
        .max(0.0);
    let mut scroll = scroll.clamp(0.0, max_s);
    if follow_focus
        && let Some(i) = focused_run
        && entry_count > 0
    {
        let i = i.min(entry_count.saturating_sub(1));
        let (row_top, row_h) = run_log_row_y(&list_layout, i, layout.run_row_h, 0.0);
        let row_bottom = row_top + row_h;
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
    let m = layout_constants(h);
    let cap_h = (typography::size(typography::H42, h) / 0.55).ceil();
    let list_layout = run_log_list_layout(panes, m.title_h, m.gap, cap_h);
    let clip_rect = [
        panes.left_content_x(),
        panes.content_y(),
        panes.left_content_w(),
        panes.content_h(),
    ];
    (0..entry_count)
        .filter_map(|i| {
            let (y, rh) = run_log_row_y(&list_layout, i, panes.run_row_h, scroll);
            intersect_rect(
                [panes.left_content_x(), y, panes.left_content_w(), rh],
                clip_rect,
            )
        })
        .collect()
}

fn pane_clip(panes: ChroniclePaneLayout) -> ChartClip {
    ChartClip {
        top: panes.content_y(),
        bottom: panes.content_y() + panes.content_h(),
    }
}

fn push_label_clipped(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    clip: ChartClip,
    label: TextLabel,
) {
    chart_primitives::push_label_clipped(out, rect, clip, label);
}

fn push_quad_clipped(out: &mut Vec<GpuInstance>, rect: [f32; 4], clip: ChartClip, c: [f32; 4]) {
    chart_quad(out, rect, clip, c);
}

fn push_section_card(
    quads: &mut Vec<GpuInstance>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    clip: ChartClip,
) {
    push_quad_clipped(
        quads,
        [x, y, w, h],
        clip,
        color::alpha(color::WALNUT_INK, 0.52),
    );
    push_quad_clipped(
        quads,
        [x, y, w, 1.0],
        clip,
        color::alpha(color::BRASS, 0.38),
    );
    let tick = 6.0_f32;
    let tc = color::alpha(color::BRASS, 0.55);
    for &(dx, dy) in &[(0.0, 0.0), (w - tick, 0.0), (0.0, h - tick), (w - tick, h - tick)] {
        push_quad_clipped(quads, [x + dx, y + dy, tick, 1.0], clip, tc);
        push_quad_clipped(quads, [x + dx, y + dy, 1.0, tick], clip, tc);
    }
}

fn push_tile_strip(
    emit: &mut ChronicleEmit<'_>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    tiles: &[crate::core::tile::Tile],
) {
    if tiles.is_empty() {
        return;
    }
    let n = tiles.len().min(14) as f32;
    let gap = 3.0;
    let tw = ((w - gap * (n - 1.0)) / n).clamp(14.0, 36.0);
    let th = tw * 1.28;
    for (i, tile) in tiles.iter().take(14).enumerate() {
        let tx = x + i as f32 * (tw + gap);
        if let Some(source) = archive_career::tile_image_source(tile) {
            let clip_rect = [
                tx,
                clip.top,
                tw,
                (clip.bottom - clip.top).max(0.0),
            ];
            let tile_rect = [tx, y, tw, th];
            if y + th <= clip.bottom
                && y >= clip.top
                && intersect_rect(tile_rect, clip_rect).is_some()
            {
                emit.images.push(ImageQuad {
                    inst: GpuInstance {
                        rect: [tx, y, tw, th],
                        color: [1.0, 1.0, 1.0, 1.0],
                        user: 0,
                    },
                    source,
                });
            }
        } else {
            push_quad_clipped(
                emit.quads,
                [tx, y, tw, th],
                clip,
                color::alpha(color::PARCHMENT, 0.75),
            );
        }
    }
}

fn push_relic_row(
    emit: &mut ChronicleEmit<'_>,
    _clip: ChartClip,
    x: f32,
    y: f32,
    relics: &[crate::core::relic::RelicId],
) {
    let icon = 28.0_f32;
    let gap = 6.0;
    for (i, rid) in relics.iter().take(10).enumerate() {
        let ix = x + i as f32 * (icon + gap);
        emit.images.push(ImageQuad {
            inst: GpuInstance {
                rect: [ix, y, icon, icon],
                color: [1.0, 1.0, 1.0, 0.96],
                user: 0,
            },
            source: ImageQuadSource::Relic(*rid),
        });
    }
}

fn push_run_log(draw: ChroniclePaneDraw<'_>, focused: Option<usize>) {
    let ChroniclePaneDraw {
        progress,
        panes,
        scroll,
        type_scale,
        emit,
    } = draw;
    let ChronicleTypeScale {
        section_px,
        body,
        caption_px,
        metrics,
        ..
    } = type_scale;
    let ChronicleLayoutMetrics { title_h, gap, .. } = metrics;
    let clip = pane_clip(panes);
    let indices = archive_career::chronicle_indices_recent_first(progress);
    let run_count = indices.len();
    let entry_count = archive_career::chronicle_list_entry_count(progress);
    let cap_h = (caption_px / 0.55).ceil();

    push_label_clipped(
        emit.labels,
        [panes.left_content_x(), panes.content_y(), panes.left_content_w(), title_h],
        clip,
        TextLabel {
            rect: [panes.left_content_x(), panes.content_y(), panes.left_content_w(), title_h],
            text: "Run log".into(),
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );

    let list_layout = run_log_list_layout(panes, title_h, gap, cap_h);
    let header_y = list_layout.list_top - scroll;
    push_label_clipped(
        emit.labels,
        [panes.left_content_x() + 8.0, header_y, panes.left_content_w() * 0.55, cap_h],
        clip,
        TextLabel {
            rect: [panes.left_content_x() + 8.0, header_y, panes.left_content_w() * 0.55, cap_h],
            text: format!("{run_count} runs · career overview"),
            color: color::alpha(color::STONE, 0.88),
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    push_label_clipped(
        emit.labels,
        [
            panes.left_content_x() + panes.left_content_w() * 0.42,
            header_y,
            panes.left_content_w() * 0.28,
            cap_h,
        ],
        clip,
        TextLabel {
            rect: [
                panes.left_content_x() + panes.left_content_w() * 0.42,
                header_y,
                panes.left_content_w() * 0.28,
                cap_h,
            ],
            text: "ANTE / FLOOR".into(),
            color: color::alpha(color::STONE, 0.75),
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    push_label_clipped(
        emit.labels,
        [
            panes.left_content_x() + panes.left_content_w() * 0.72,
            header_y,
            panes.left_content_w() * 0.26,
            cap_h,
        ],
        clip,
        TextLabel {
            rect: [
                panes.left_content_x() + panes.left_content_w() * 0.72,
                header_y,
                panes.left_content_w() * 0.26,
                cap_h,
            ],
            text: "SCORE".into(),
            color: color::alpha(color::STONE, 0.75),
            font_px: Some(caption_px),
            align: TextAlign::Right,
            ..Default::default()
        },
    );

    let runs_top = list_layout.list_top + list_layout.header_band_h + list_layout.summary_row_h;
    chart_primitives::push_quad(
        emit.quads,
        [panes.left_content_x(), runs_top - scroll - 1.0, panes.left_content_w(), 1.0],
        color::alpha(color::BRASS, 0.2),
    );

    if run_count == 0 {
        let row_y = list_layout.list_top + list_layout.header_band_h - scroll;
        push_label_clipped(
            emit.labels,
            [panes.left_content_x(), row_y, panes.left_content_w(), panes.run_row_h * 2.0],
            clip,
            TextLabel {
                rect: [panes.left_content_x(), row_y, panes.left_content_w(), panes.run_row_h * 2.0],
                text: "Finish a non-tutorial run to add entries here.".into(),
                color: color::alpha(color::STONE, 0.95),
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        return;
    }

    for list_i in 0..entry_count {
        let (row_y, row_h) = run_log_row_y(&list_layout, list_i, panes.run_row_h, scroll);
        if intersect_rect(
            [panes.left_content_x(), row_y, panes.left_content_w(), row_h],
            [
                panes.left_content_x(),
                clip.top,
                panes.left_content_w(),
                clip.bottom - clip.top,
            ],
        )
        .is_none()
        {
            continue;
        }
        let selected = focused == Some(list_i);
        if selected {
            push_quad_clipped(
                emit.quads,
                [panes.left_content_x(), row_y, panes.left_content_w(), row_h],
                clip,
                color::alpha(color::WALNUT_RAISED, 0.92),
            );
            push_quad_clipped(
                emit.quads,
                [panes.left_content_x(), row_y, 3.0, row_h],
                clip,
                color::alpha(color::CHAMPAGNE, 0.75),
            );
        }

        if list_i == 0 {
            push_label_clipped(
                emit.labels,
                [panes.left_content_x() + 8.0, row_y + 4.0, panes.left_content_w() * 0.5, row_h - 6.0],
                clip,
                TextLabel {
                    rect: [panes.left_content_x() + 8.0, row_y + 4.0, panes.left_content_w() * 0.5, row_h - 6.0],
                    text: "Summary".into(),
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
            continue;
        }

        let hist_idx = indices[list_i - 1];
        let Some(rec) = progress.run_history.get(hist_idx) else {
            continue;
        };
        let display = archive_career::chronicle_display_run_number(list_i, progress).unwrap_or(0);
        let outcome_color = archive_career::chronicle_run_outcome_color(rec);
        let line1_h = (body / 0.55).ceil() + 2.0;
        let line2_h = if archive_career::chronicle_run_log_boss_line(rec).is_some() {
            (caption_px / 0.55).ceil() + 2.0
        } else {
            0.0
        };
        let meta_h = (caption_px / 0.55).ceil() + 4.0;
        let dominant = archive_career::run_dominant_yaku(rec);
        let pill_w = dominant.map(|yk| {
            (yk.name().len() as f32 * caption_px * 0.42 + 14.0).min(panes.left_content_w() * 0.30)
        });
        let line1_w = panes.left_content_w() * 0.62;

        push_label_clipped(
            emit.labels,
            [panes.left_content_x() + 8.0, row_y + 2.0, line1_w, line1_h],
            clip,
            TextLabel {
                rect: [panes.left_content_x() + 8.0, row_y + 2.0, line1_w, line1_h],
                text: archive_career::chronicle_run_log_line1(progress, display, rec),
                color: if selected {
                    color::CHAMPAGNE
                } else {
                    outcome_color
                },
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        if let Some(boss) = archive_career::chronicle_run_log_boss_line(rec) {
            push_label_clipped(
                emit.labels,
                [
                    panes.left_content_x() + 8.0,
                    row_y + line1_h,
                    panes.left_content_w() * 0.62,
                    line2_h,
                ],
                clip,
                TextLabel {
                    rect: [
                        panes.left_content_x() + 8.0,
                        row_y + line1_h,
                        panes.left_content_w() * 0.62,
                        line2_h,
                    ],
                    text: boss,
                    color: color::alpha(color::STONE, 0.92),
                    font_px: Some(caption_px),
                    align: TextAlign::Left,
                    ..Default::default()
                },
            );
        }
        let meta_y = row_y + line1_h + line2_h;
        push_label_clipped(
            emit.labels,
            [
                panes.left_content_x() + 8.0,
                meta_y,
                panes.left_content_w() * 0.48,
                meta_h,
            ],
            clip,
            TextLabel {
                rect: [
                    panes.left_content_x() + 8.0,
                    meta_y,
                    panes.left_content_w() * 0.48,
                    meta_h,
                ],
                text: archive_career::chronicle_floor_shorthand(rec),
                color: color::alpha(color::STONE, 0.9),
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        push_label_clipped(
            emit.labels,
            [
                panes.left_content_x() + panes.left_content_w() * 0.52,
                meta_y,
                panes.left_content_w() * 0.44,
                meta_h,
            ],
            clip,
            TextLabel {
                rect: [
                    panes.left_content_x() + panes.left_content_w() * 0.52,
                    meta_y,
                    panes.left_content_w() * 0.44,
                    meta_h,
                ],
                text: archive_career::format_score(rec.total_score_earned),
                color: if selected {
                    color::CHAMPAGNE
                } else {
                    color::alpha(color::PARCHMENT, 0.94)
                },
                font_px: Some(body),
                align: TextAlign::Right,
                ..Default::default()
            },
        );

        if let Some(yk) = dominant {
            let pill_w = pill_w.unwrap_or(48.0);
            let px = (panes.left_content_x() + panes.left_content_w() * 0.50 - pill_w - 4.0)
                .max(panes.left_content_x() + panes.left_content_w() * 0.34);
            let py = meta_y;
            push_quad_clipped(
                emit.quads,
                [px, py, pill_w, cap_h + 4.0],
                clip,
                archive_career::yaku_pill_color(yk),
            );
            push_label_clipped(
                emit.labels,
                [px + 4.0, py + 1.0, pill_w - 8.0, cap_h + 2.0],
                clip,
                TextLabel {
                    rect: [px + 4.0, py + 1.0, pill_w - 8.0, cap_h + 2.0],
                    text: yk.name().into(),
                    color: color::alpha(color::PARCHMENT, 0.95),
                    font_px: Some(caption_px * 0.92),
                    align: TextAlign::Center,
                    ..Default::default()
                },
            );
        }
    }
}

fn push_section_title(
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: &str,
    section_px: f32,
) {
    push_label_clipped(
        labels,
        [x, y, w, h],
        clip,
        TextLabel {
            rect: [x, y, w, h],
            text: text.into(),
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
}

fn push_career_pane(h: f32, _w: f32, draw: ChroniclePaneDraw<'_>) {
    let ChroniclePaneDraw {
        progress,
        panes,
        scroll,
        type_scale,
        mut emit,
    } = draw;
    let ChronicleTypeScale {
        section_px,
        body,
        caption_px,
        metrics,
    } = type_scale;
    let clip = pane_clip(panes);
    let cap_h = (caption_px / 0.55).ceil();
    let val_h = (body / 0.55).ceil();
    let runs = serious_runs_chronological(progress);
    let mut doc_y = 0.0_f32;
    let rx = panes.right_content_x();
    let rw = panes.right_content_w();
    let ry = |dy: f32| panes.content_y() + dy - scroll;

    push_section_title(
        emit.labels,
        clip,
        rx,
        ry(doc_y),
        rw,
        metrics.title_h,
        "Career",
        section_px,
    );
    doc_y += metrics.title_h + metrics.gap * 0.5;

    let kpis = archive_career::career_kpi_strip(progress);
    let kpi_gap = metrics.gap * 0.35;
    let kpi_w = (rw - kpi_gap * (KPI_COUNT as f32 - 1.0)) / KPI_COUNT as f32;
    for (i, kpi) in kpis.iter().take(KPI_COUNT).enumerate() {
        let kx = rx + i as f32 * (kpi_w + kpi_gap);
        let ky = ry(doc_y);
        push_section_card(emit.quads, kx, ky, kpi_w, metrics.kpi_h, clip);
        push_label_clipped(
            emit.labels,
            [kx + 8.0, ky + 10.0, kpi_w - 12.0, cap_h],
            clip,
            TextLabel {
                rect: [kx + 8.0, ky + 10.0, kpi_w - 12.0, cap_h],
                text: kpi.label.into(),
                color: color::STONE,
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        push_label_clipped(
            emit.labels,
            [kx + 8.0, ky + cap_h + 12.0, kpi_w - 12.0, val_h],
            clip,
            TextLabel {
                rect: [kx + 8.0, ky + cap_h + 8.0, kpi_w - 12.0, val_h],
                text: kpi.value.clone(),
                color: color::CHAMPAGNE,
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        if let Some(ref d) = kpi.detail {
            let detail_y = ky + cap_h + 12.0 + val_h + 6.0;
            if detail_y + cap_h <= ky + metrics.kpi_h - 4.0 {
                push_label_clipped(
                    emit.labels,
                    [kx + 8.0, detail_y, kpi_w - 12.0, cap_h],
                    clip,
                    TextLabel {
                        rect: [kx + 8.0, detail_y, kpi_w - 12.0, cap_h],
                        text: d.clone(),
                        color: color::alpha(color::STONE, 0.88),
                        font_px: Some(caption_px),
                        align: TextAlign::Left,
                        ..Default::default()
                    },
                );
            }
        }
    }
    doc_y += metrics.kpi_h + metrics.gap;

    if let Some(rec) = archive_career::career_signature_record(progress) {
        let tile_strip_h = if rec.best_hand_tiles.is_empty() {
            0.0
        } else {
            38.0
        };
        let hero_h = (8.0 + cap_h + 8.0 + val_h + 8.0 + val_h + 12.0 + tile_strip_h + 10.0)
            .max(metrics.hero_h);
        let hy = ry(doc_y);
        push_section_card(emit.quads, rx, hy, rw, hero_h, clip);
        push_label_clipped(
            emit.labels,
            [rx + 10.0, hy + 8.0, rw - 16.0, cap_h],
            clip,
            TextLabel {
                rect: [rx + 10.0, hy + 8.0, rw - 16.0, cap_h],
                text: "Signature hand".into(),
                color: color::STONE,
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        push_label_clipped(
            emit.labels,
            [rx + 10.0, hy + cap_h + 10.0, rw - 16.0, val_h * 1.15],
            clip,
            TextLabel {
                rect: [rx + 10.0, hy + cap_h + 10.0, rw - 16.0, val_h * 1.15],
                text: rec.best_structure_name.clone(),
                color: color::PARCHMENT,
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        push_label_clipped(
            emit.labels,
            [rx + 10.0, hy + cap_h + 12.0 + val_h * 1.15, rw - 16.0, val_h],
            clip,
            TextLabel {
                rect: [rx + 10.0, hy + cap_h + 12.0 + val_h * 1.15, rw - 16.0, val_h],
                text: archive_career::format_score(rec.best_structure_score),
                color: color::CHAMPAGNE,
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        if tile_strip_h > 0.0 {
            push_tile_strip(
                &mut emit,
                clip,
                rx + 10.0,
                hy + cap_h + 12.0 + val_h * 1.15 + val_h + 10.0,
                rw - 20.0,
                &rec.best_hand_tiles,
            );
        }
        doc_y += hero_h + metrics.gap;
    }

    let tiles = archive_career::career_tiles(progress);
    let tile_gap = metrics.gap * 0.4;
    let tile_n = tiles.len().min(4);
    let cols = if tile_n <= 2 { tile_n.max(1) as f32 } else { 2.0 };
    let rows = ((tile_n + 1) / 2).max(1) as f32;
    let tile_w = (rw - tile_gap * (cols - 1.0)) / cols;
    let th = highlight_tile_height(h);
    for (i, tile) in tiles.iter().take(4).enumerate() {
        let col = if tile_n <= 2 {
            i as f32
        } else {
            (i % 2) as f32
        };
        let row = if tile_n <= 2 { 0.0 } else { (i / 2) as f32 };
        let tx = rx + col * (tile_w + tile_gap);
        let ty = ry(doc_y) + row * (th + tile_gap);
        push_section_card(emit.quads, tx, ty, tile_w, th, clip);
        push_label_clipped(
            emit.labels,
            [tx + 8.0, ty + 6.0, tile_w - 12.0, cap_h],
            clip,
            TextLabel {
                rect: [tx + 8.0, ty + 6.0, tile_w - 12.0, cap_h],
                text: tile.label.into(),
                color: color::STONE,
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        let value_y = ty + cap_h + 10.0;
        push_label_clipped(
            emit.labels,
            [tx + 8.0, value_y, tile_w - 12.0, val_h],
            clip,
            TextLabel {
                rect: [tx + 8.0, value_y, tile_w - 12.0, val_h],
                text: tile.value.clone(),
                color: color::PARCHMENT,
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        if let Some(ref detail) = tile.detail {
            let detail_y = value_y + val_h + 4.0;
            if detail_y + cap_h <= ty + th - 4.0 {
                push_label_clipped(
                    emit.labels,
                    [tx + 8.0, detail_y, tile_w - 12.0, cap_h],
                    clip,
                    TextLabel {
                        rect: [tx + 8.0, detail_y, tile_w - 12.0, cap_h],
                        text: detail.clone(),
                        color: color::alpha(color::STONE, 0.9),
                        font_px: Some(caption_px),
                        align: TextAlign::Left,
                        ..Default::default()
                    },
                );
            }
        }
    }
    doc_y += th * rows + tile_gap * (rows - 1.0).max(0.0) + metrics.gap;

    if runs.is_empty() {
        return;
    }

    let grid_line = color::alpha(color::UMBER, 0.45);
    let label_w = (rw * 0.38).min(180.0);

    push_section_title(
        emit.labels,
        clip,
        rx,
        ry(doc_y),
        rw,
        metrics.title_h,
        "Score history",
        section_px,
    );
    doc_y += metrics.title_h + metrics.gap * 0.5;
    let chart_top = ry(doc_y);
    let (peaks, max_score) = score_chart_columns(&runs);
    push_vbar_chart(
        emit.quads,
        emit.labels,
        clip,
        rx,
        chart_top,
        rw,
        metrics.chart_h,
        &peaks,
        max_score,
        color::alpha(color::CHAMPAGNE, 0.78),
        true,
        grid_line,
        caption_px,
    );
    doc_y += metrics.chart_h + cap_h + metrics.gap;

    push_section_title(
        emit.labels,
        clip,
        rx,
        ry(doc_y),
        rw,
        metrics.title_h,
        "Score distribution",
        section_px,
    );
    doc_y += metrics.title_h + metrics.gap * 0.5;
    let buckets = archive_career::score_distribution_buckets(progress);
    let max_b = buckets.iter().map(|b| b.count).max().unwrap_or(1).max(1);
    let total_runs = runs.len().max(1) as f32;
    for (i, b) in buckets.iter().enumerate() {
        let row_top = ry(doc_y) + i as f32 * metrics.bar_row_h;
        let pct = (b.count as f32 / total_runs * 100.0).round() as u32;
        push_hbar_row(
            emit.quads,
            emit.labels,
            clip,
            rx,
            row_top,
            rw,
            metrics.bar_row_h,
            b.label,
            b.count,
            max_b,
            label_w,
            color::alpha(color::STONE, 0.96),
            color::alpha(color::WALNUT_SOFT, 0.8),
            color::alpha(color::PARCHMENT, 0.94),
            caption_px,
            body,
            Some(&format!(" ({pct}%)")),
        );
    }
    doc_y += buckets.len() as f32 * metrics.bar_row_h + metrics.gap;

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

    push_section_title(
        emit.labels,
        clip,
        rx,
        ry(doc_y),
        rw,
        metrics.title_h,
        "Outcomes",
        section_px,
    );
    doc_y += metrics.title_h + metrics.gap * 0.5;
    let bar_y = ry(doc_y);
    push_stacked_bar(
        emit.quads,
        clip,
        rx,
        bar_y,
        rw,
        metrics.bar_row_h * 0.88,
        wins as f32 / total_o as f32,
        color::alpha(color::JADE, 0.85),
        color::alpha(color::RUBY, 0.85),
        grid_line,
    );
    doc_y += metrics.bar_row_h + metrics.gap * 0.35;
    push_label_clipped(
        emit.labels,
        [rx, ry(doc_y), rw, cap_h],
        clip,
        TextLabel {
            rect: [rx, ry(doc_y), rw, cap_h],
            text: format!("Win {w_pct}%  ·  Loss {l_pct}%"),
            color: color::alpha(color::PARCHMENT, 0.94),
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    doc_y += cap_h + metrics.gap;

    let mut yaku: Vec<(YakuKind, u32)> = progress
        .yaku_times_scored
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    yaku.sort_by(|a, b| b.1.cmp(&a.1));
    let max_y = yaku.first().map(|(_, c)| *c).unwrap_or(1).max(1);
    let total_yaku: u32 = progress.yaku_times_scored.values().sum();

    push_section_title(
        emit.labels,
        clip,
        rx,
        ry(doc_y),
        rw,
        metrics.title_h,
        "Yaku frequency (top 6)",
        section_px,
    );
    doc_y += metrics.title_h + metrics.gap * 0.5;
    for (yk, count) in yaku.into_iter().take(6) {
        let row_top = ry(doc_y);
        let pct = if total_yaku > 0 {
            (count as f32 / total_yaku as f32 * 100.0).round() as u32
        } else {
            0
        };
        push_hbar_row(
            emit.quads,
            emit.labels,
            clip,
            rx,
            row_top,
            rw,
            metrics.bar_row_h,
            yk.name(),
            count,
            max_y,
            label_w,
            color::alpha(color::STONE, 0.96),
            archive_career::yaku_pill_color(yk),
            color::alpha(color::PARCHMENT, 0.94),
            caption_px,
            body,
            Some(&format!(" ({pct}%)")),
        );
        doc_y += metrics.bar_row_h + 4.0;
    }
    doc_y += metrics.gap;

    let footer = archive_career::career_footer_stats(progress);
    if !footer.is_empty() {
        let fy = ry(doc_y);
        push_section_card(emit.quads, rx, fy, rw, metrics.footer_h, clip);
        let slot = rw / footer.len() as f32;
        for (i, stat) in footer.iter().enumerate() {
            let sx = rx + i as f32 * slot;
            push_label_clipped(
                emit.labels,
                [sx, fy + 4.0, slot, cap_h + 2.0],
                clip,
                TextLabel {
                    rect: [sx, fy + 4.0, slot, cap_h + 2.0],
                    text: format!("{} {}", stat.icon, stat.value),
                    color: color::CHAMPAGNE,
                    font_px: Some(caption_px),
                    align: TextAlign::Center,
                    ..Default::default()
                },
            );
            push_label_clipped(
                emit.labels,
                [sx, fy + cap_h + 8.0, slot, cap_h],
                clip,
                TextLabel {
                    rect: [sx, fy + cap_h + 8.0, slot, cap_h],
                    text: stat.label.into(),
                    color: color::alpha(color::STONE, 0.85),
                    font_px: Some(caption_px * 0.9),
                    align: TextAlign::Center,
                    ..Default::default()
                },
            );
        }
    }
}

fn push_run_detail_pane(draw: ChroniclePaneDraw<'_>, list_index: usize) {
    let ChroniclePaneDraw {
        progress,
        panes,
        scroll,
        type_scale,
        mut emit,
    } = draw;
    let ChronicleTypeScale {
        section_px,
        body,
        caption_px,
        metrics,
    } = type_scale;
    let clip = pane_clip(panes);
    let cap_h = (caption_px / 0.55).ceil();
    let val_h = (body / 0.55).ceil();

    let Some(hist_idx) = archive_career::chronicle_hist_index_at_list(list_index, progress) else {
        return;
    };
    let Some(rec) = progress.run_history.get(hist_idx) else {
        return;
    };
    let display = archive_career::chronicle_display_run_number(list_index, progress).unwrap_or(0);
    let model = archive_career::run_detail_model(progress, display, rec);

    let rx = panes.right_content_x();
    let rw = panes.right_content_w();
    let mut doc_y = 0.0_f32;
    let ry = |dy: f32| panes.content_y() + dy - scroll;

    push_label_clipped(
        emit.labels,
        [rx, ry(doc_y), rw, metrics.title_h],
        clip,
        TextLabel {
            rect: [rx, ry(doc_y), rw, metrics.title_h],
            text: model.heading.clone(),
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    doc_y += metrics.title_h + 4.0;
    push_label_clipped(
        emit.labels,
        [rx, ry(doc_y), rw, cap_h],
        clip,
        TextLabel {
            rect: [rx, ry(doc_y), rw, cap_h],
            text: model.timestamp_line.clone(),
            color: color::alpha(color::STONE, 0.9),
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    doc_y += cap_h + metrics.gap * 0.75;

    let hy = ry(doc_y);
    push_section_card(emit.quads, rx, hy, rw, metrics.hero_h, clip);
    let sig_label = if model.tiles_representative {
        "Signature hand (representative)"
    } else {
        "Signature hand"
    };
    push_label_clipped(
        emit.labels,
        [rx + 10.0, hy + 8.0, rw - 16.0, cap_h],
        clip,
        TextLabel {
            rect: [rx + 10.0, hy + 8.0, rw - 16.0, cap_h],
            text: sig_label.into(),
            color: color::STONE,
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    push_label_clipped(
        emit.labels,
        [rx + 10.0, hy + cap_h + 8.0, rw - 16.0, val_h],
        clip,
        TextLabel {
            rect: [rx + 10.0, hy + cap_h + 8.0, rw - 16.0, val_h],
            text: format!(
                "{} · {}",
                model.signature_name,
                archive_career::format_score(model.signature_score)
            ),
            color: color::PARCHMENT,
            font_px: Some(body),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    push_tile_strip(
        &mut emit,
        clip,
        rx + 10.0,
        hy + cap_h + val_h + 14.0,
        rw - 20.0,
        &model.tiles,
    );
    doc_y += metrics.hero_h + metrics.gap;

    if !model.yaku_rows.is_empty() {
        push_section_title(
            emit.labels,
            clip,
            rx,
            ry(doc_y),
            rw,
            metrics.title_h,
            "Yaku breakdown",
            section_px,
        );
        doc_y += metrics.title_h + metrics.gap * 0.4;
        let max_y = model.yaku_rows.first().map(|(_, c)| *c).unwrap_or(1).max(1);
        let label_w = rw * 0.38;
        for (yk, count) in model.yaku_rows.iter().take(8) {
            let row_top = ry(doc_y);
            push_hbar_row(
                emit.quads,
                emit.labels,
                clip,
                rx,
                row_top,
                rw,
                metrics.bar_row_h,
                yk.name(),
                *count,
                max_y,
                label_w,
                color::alpha(color::STONE, 0.96),
                archive_career::yaku_pill_color(*yk),
                color::alpha(color::PARCHMENT, 0.94),
                caption_px,
                body,
                None,
            );
            doc_y += metrics.bar_row_h + 3.0;
        }
        doc_y += metrics.gap * 0.5;
    }

    push_section_title(
        emit.labels,
        clip,
        rx,
        ry(doc_y),
        rw,
        metrics.title_h,
        "Score",
        section_px,
    );
    doc_y += metrics.title_h + metrics.gap * 0.35;
    for line in &model.score_lines {
        push_label_clipped(
            emit.labels,
            [rx, ry(doc_y), rw, metrics.line_h],
            clip,
            TextLabel {
                rect: [rx, ry(doc_y), rw, metrics.line_h],
                text: line.clone(),
                color: color::alpha(color::PARCHMENT, 0.94),
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += metrics.line_h;
    }
    doc_y += metrics.gap * 0.5;

    if model.ante_scores.len() >= 2 {
        push_section_title(
            emit.labels,
            clip,
            rx,
            ry(doc_y),
            rw,
            metrics.title_h,
            "Ante progression",
            section_px,
        );
        doc_y += metrics.title_h + metrics.gap * 0.4;
        let chart_top = ry(doc_y);
        let max_s = model
            .ante_scores
            .iter()
            .map(|(_, s)| *s)
            .max()
            .unwrap_or(1)
            .max(1);
        let samples: Vec<f32> = model
            .ante_scores
            .iter()
            .map(|(_, s)| *s as f32 / max_s as f32)
            .collect();
        push_sparkline(
            emit.quads,
            clip,
            rx,
            chart_top,
            rw,
            metrics.chart_h,
            &samples,
            color::alpha(color::CHAMPAGNE, 0.9),
            color::alpha(color::UMBER, 0.5),
        );
        doc_y += metrics.chart_h + metrics.gap;
    }

    if !model.timeline.is_empty() {
        push_section_title(
            emit.labels,
            clip,
            rx,
            ry(doc_y),
            rw,
            metrics.title_h,
            "Encounter history",
            section_px,
        );
        doc_y += metrics.title_h + metrics.gap * 0.35;
        for (ante, blind, note) in &model.timeline {
            push_label_clipped(
                emit.labels,
                [rx, ry(doc_y), rw, metrics.line_h * 0.9],
                clip,
                TextLabel {
                    rect: [rx, ry(doc_y), rw, metrics.line_h * 0.9],
                    text: format!("Ante {ante} · {blind} · {note}"),
                    color: color::alpha(color::STONE, 0.92),
                    font_px: Some(caption_px),
                    align: TextAlign::Left,
                    ..Default::default()
                },
            );
            doc_y += metrics.line_h * 0.9;
        }
        doc_y += metrics.gap * 0.5;
    }

    if !rec.relics_owned.is_empty() {
        push_section_title(
            emit.labels,
            clip,
            rx,
            ry(doc_y),
            rw,
            metrics.title_h,
            &format!("Relics ({})", rec.relics_owned.len()),
            section_px,
        );
        doc_y += metrics.title_h + metrics.gap * 0.35;
        push_relic_row(&mut emit, clip, rx, ry(doc_y), &rec.relics_owned);
        doc_y += 36.0 + metrics.gap * 0.5;
    }

    if !rec.consumables_owned.is_empty() {
        let names: Vec<String> = rec
            .consumables_owned
            .iter()
            .map(|c| format!("{c:?}"))
            .collect();
        push_label_clipped(
            emit.labels,
            [rx, ry(doc_y), rw, metrics.line_h],
            clip,
            TextLabel {
                rect: [rx, ry(doc_y), rw, metrics.line_h],
                text: format!("Consumables: {}", names.join(", ")),
                color: color::alpha(color::STONE, 0.9),
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += metrics.line_h + metrics.gap * 0.5;
    }

    if let Some(kind) = rec.memorial_kind {
        push_label_clipped(
            emit.labels,
            [rx, ry(doc_y), rw, metrics.line_h],
            clip,
            TextLabel {
                rect: [rx, ry(doc_y), rw, metrics.line_h],
                text: format!("Memorial: {}", kind.name()),
                color: color::alpha(color::ANTIQUE, 0.95),
                font_px: Some(body),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        doc_y += metrics.line_h + metrics.gap;
    }

    let fy = ry(doc_y);
    push_section_card(emit.quads, rx, fy, rw, metrics.footer_h, clip);
    let n = model.footer.len().max(1) as f32;
    let slot = rw / n;
    for (i, (label, value)) in model.footer.iter().enumerate() {
        let sx = rx + i as f32 * slot;
        push_label_clipped(
            emit.labels,
            [sx, fy + 4.0, slot, cap_h + 2.0],
            clip,
            TextLabel {
                rect: [sx, fy + 4.0, slot, cap_h + 2.0],
                text: value.clone(),
                color: color::CHAMPAGNE,
                font_px: Some(caption_px),
                align: TextAlign::Center,
                ..Default::default()
            },
        );
        push_label_clipped(
            emit.labels,
            [sx, fy + cap_h + 8.0, slot, cap_h],
            clip,
            TextLabel {
                rect: [sx, fy + cap_h + 8.0, slot, cap_h],
                text: label.clone(),
                color: color::alpha(color::STONE, 0.85),
                font_px: Some(caption_px * 0.9),
                align: TextAlign::Center,
                ..Default::default()
            },
        );
    }
}

pub fn push_chronicle_dashboard(
    w: f32,
    h: f32,
    panel: [f32; 4],
    view: ChronicleView,
    progress: &PlayerProgress,
    out_quads: &mut Vec<GpuInstance>,
    out_labels: &mut Vec<TextLabel>,
    out_images: &mut Vec<ImageQuad>,
) {
    let panes = chronicle_pane_layout(w, h, panel);
    let metrics = layout_constants(h);
    let type_scale = ChronicleTypeScale {
        section_px: typography::size(typography::H28, h),
        body: metrics.body,
        caption_px: typography::size(typography::H42, h),
        metrics,
    };

    push_ledger_sheet(
        out_quads,
        panes.inner_x,
        panes.inner_y,
        panes.inner_w,
        panes.inner_h,
    );
    let (run_log_rect, career_rect) = chronicle_pane_rects(w, h, panel);
    push_pane_focus_ring(out_quads, run_log_rect, view.focused_pane == ChronicleScrollPane::RunLog);
    push_pane_focus_ring(out_quads, career_rect, view.focused_pane == ChronicleScrollPane::Career);
    chart_primitives::push_quad(
        out_quads,
        [
            panes.left_x + panes.left_w,
            panes.inner_y,
            panes.gutter,
            panes.inner_h,
        ],
        color::alpha(color::BRASS, 0.16),
    );

    let emit = ChronicleEmit {
        quads: out_quads,
        labels: out_labels,
        images: out_images,
    };

    push_run_log(
        ChroniclePaneDraw {
            progress,
            panes,
            scroll: view.run_log_scroll,
            type_scale,
            emit: ChronicleEmit {
                quads: emit.quads,
                labels: emit.labels,
                images: emit.images,
            },
        },
        view.focused_run,
    );

    let show_summary = view.focused_run.is_none() || view.focused_run == Some(0);
    if show_summary {
        push_career_pane(
            h,
            w,
            ChroniclePaneDraw {
                progress,
                panes,
                scroll: view.career_scroll,
                type_scale,
                emit,
            },
        );
    } else if let Some(list_i) = view.focused_run {
        push_run_detail_pane(
            ChroniclePaneDraw {
                progress,
                panes,
                scroll: view.career_scroll,
                type_scale,
                emit,
            },
            list_i,
        );
    }
}

fn push_pane_focus_ring(out: &mut Vec<GpuInstance>, rect: [f32; 4], active: bool) {
    if !active {
        return;
    }
    let [x, y, w, h] = rect;
    let b = 2.0;
    let c = color::alpha(color::CHAMPAGNE, 0.72);
    chart_primitives::push_quad(out, [x, y, w, b], c);
    chart_primitives::push_quad(out, [x, y + h - b, w, b], c);
    chart_primitives::push_quad(out, [x, y, b, h], c);
    chart_primitives::push_quad(out, [x + w - b, y, b, h], c);
}

fn push_ledger_sheet(
    out: &mut Vec<GpuInstance>,
    inner_x: f32,
    inner_y: f32,
    inner_w: f32,
    inner_h: f32,
) {
    let b = FRAME_BORDER_PX;
    chart_primitives::push_quad(
        out,
        [
            inner_x - b,
            inner_y - b,
            inner_w + b * 2.0,
            inner_h + b * 2.0,
        ],
        color::alpha(color::BRASS, 0.5),
    );
    chart_primitives::push_quad(
        out,
        [inner_x, inner_y, inner_w, inner_h],
        color::alpha(color::WALNUT_DEEP, 0.95),
    );
}

pub fn chronicle_dim_gradient(panel: [f32; 4]) -> GradientQuadInstance {
    GradientQuadInstance {
        rect: panel,
        color: color::alpha(color::WALNUT_INK, 0.94),
        feather: [0.14, 0.0, 0.0, 0.0],
    }
}
