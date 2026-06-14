//! Split-pane Chronicle ledger for the Archive tab: run log (left) + career stats (right).

use crate::core::progression::{PlayerProgress, RunRecord};
use crate::render::doc_tile_camera::TOP_DOWN_TILE_ROTATION;
use crate::render::draw_cmd::{ImageQuad, ImageQuadSource, ShowcaseTilePlacement};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, GradientQuadInstance, TextAlign, TextLabel};
use crate::scenes::archive_career;
use crate::ui::chart_primitives::{
    self, ChartClip, LedgerPanelStyle, push_colored_label_clipped, push_ledger_panel_clipped,
    push_quad_clipped as chart_quad, push_sparkline, push_yaku_hbar_row,
    push_yaku_pill,
};
use crate::ui::chronicle_charts;
use crate::ui::clip::intersect_rect;
use crate::ui::tooltip::push_tooltip_frame_quads;
/// Run log is a narrow receipt; career pane needs the width for KPIs and charts.
const LEFT_PANE_FRAC: f32 = 0.30;
const KPI_COUNT: usize = 4;
/// Focus-ring stroke; content and [`ChroniclePaneLayout`] insets sit inside this border.
const PANE_RING_BORDER: f32 = 2.0;

/// Xanh Mono for receipt columns, KPI values, and other tabular Chronicle copy.
#[inline]
fn tabular(lbl: TextLabel) -> TextLabel {
    TextLabel { mono: true, ..lbl }
}

#[derive(Clone, Copy, Debug)]
struct KpiColumnLayout {
    kpi_w: f32,
    gap: f32,
}

fn kpi_column_layout(rw: f32, gap: f32) -> KpiColumnLayout {
    KpiColumnLayout {
        kpi_w: (rw - gap * (KPI_COUNT as f32 - 1.0)) / KPI_COUNT as f32,
        gap,
    }
}

fn kpi_column_rect(rx: f32, layout: KpiColumnLayout, col: usize) -> (f32, f32) {
    let x = rx + col as f32 * (layout.kpi_w + layout.gap);
    (x, layout.kpi_w)
}

fn kpi_span_rect(rx: f32, layout: KpiColumnLayout, start_col: usize, count: usize) -> (f32, f32) {
    let x = rx + start_col as f32 * (layout.kpi_w + layout.gap);
    let w = layout.kpi_w * count as f32 + layout.gap * count.saturating_sub(1) as f32;
    (x, w)
}

fn insights_signature_inner_w(rw: f32, gap: f32, inset: f32) -> f32 {
    let cols = kpi_column_layout(rw, gap);
    let (_, sig_w) = kpi_span_rect(0.0, cols, 0, 2);
    (sig_w - inset * 2.0).max(60.0)
}

/// Chronicle typography and rhythm — all sizes are `window_h` fractions (see [`typography`]).
mod rhythm {
    use crate::render::theme::typography;

    /// Pane titles ("Run log", "Career").
    pub const PANE_TITLE: f32 = typography::H28;
    /// Run lines, KPI values, chart labels.
    pub const BODY: f32 = typography::H36;
    /// Column headers, ante, KPI captions, subsection heads.
    pub const CAPTION: f32 = typography::H42;
    /// Fine print inside cards.
    pub const MICRO: f32 = typography::H45;

    pub const SECTION_GAP: f32 = 1.0 / 48.0;
    pub const ROW_PAD: f32 = 1.0 / 128.0;
    /// Gap between a KPI caption and its value.
    pub const KPI_STACK_GAP: f32 = 1.0 / 96.0;
    pub const PAD_X: f32 = 1.0 / 64.0;
    pub const PAD_Y: f32 = 1.0 / 56.0;
    pub const GUTTER: f32 = 1.0 / 80.0;
    pub const SHEET_INSET: f32 = 1.0 / 140.0;
    pub const CARD_INSET: f32 = 1.0 / 90.0;

    /// Ledger band below tab chrome (`window_h` fraction).
    pub const BAND_TOP: f32 = 1.0 / 11.0;
    /// Space reserved above the footer hint.
    pub const BAND_BOTTOM: f32 = 1.0 / 18.0;
    /// Horizontal safe inset for the full-bleed panel.
    pub const PANEL_INSET_X: f32 = 1.0 / 96.0;

    #[inline]
    pub fn line_h(font_px: f32) -> f32 {
        (font_px / 0.55).ceil() + 1.0
    }

    #[inline]
    pub fn card_inset(h: f32) -> f32 {
        (h * CARD_INSET).max(4.0)
    }

    #[inline]
    pub fn stack_gap(h: f32) -> f32 {
        (h * KPI_STACK_GAP).max(2.0)
    }

    #[inline]
    pub fn section_gap(h: f32) -> f32 {
        (h * SECTION_GAP).max(5.0)
    }
}

/// Full-bleed Chronicle panel rect `[x, y, w, h]` for the Archive tab.
pub fn chronicle_panel_rect(w: f32, h: f32) -> [f32; 4] {
    let inset_x = (h * rhythm::PANEL_INSET_X).max(6.0);
    let band_top = h * rhythm::BAND_TOP;
    let band_bottom = h * (1.0 - rhythm::BAND_BOTTOM);
    let band_h = (band_bottom - band_top).max(1.0);
    [inset_x, band_top, w - inset_x * 2.0, band_h]
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ChronicleView {
    pub focused_run: Option<usize>,
    pub run_log_scroll: f32,
    pub career_scroll: f32,
}

/// Chronicle ledger column target for scroll deltas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChronicleScrollPane {
    #[default]
    RunLog,
    Career,
}

/// Screen rects for run log (left) and career/detail (right) columns — outer bounds for the focus ring.
pub fn chronicle_pane_rects(w: f32, h: f32, panel: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let panes = chronicle_pane_layout(w, h, panel);
    (panes.left_pane_rect(), panes.right_pane_rect())
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
    let [lx, py, lw, ph] = panes.left_pane_rect();
    if cy < py || cy > py + ph {
        return None;
    }
    let [rx, _, rw, _] = panes.right_pane_rect();
    if cx >= lx && cx < lx + lw {
        Some(ChronicleScrollPane::RunLog)
    } else if cx >= rx && cx < rx + rw {
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
    fn pane_content_inset(&self) -> f32 {
        PANE_RING_BORDER
    }

    #[inline]
    pub fn left_pane_rect(&self) -> [f32; 4] {
        [self.left_x, self.inner_y, self.left_w, self.inner_h]
    }

    #[inline]
    pub fn right_pane_rect(&self) -> [f32; 4] {
        [self.right_x, self.inner_y, self.right_w, self.inner_h]
    }

    #[inline]
    pub fn content_y(&self) -> f32 {
        self.inner_y + self.pane_content_inset() + self.pad_y
    }

    #[inline]
    pub fn content_h(&self) -> f32 {
        (self.inner_h - (self.pane_content_inset() + self.pad_y) * 2.0).max(1.0)
    }

    #[inline]
    pub fn left_content_x(&self) -> f32 {
        self.left_x + self.pane_content_inset() + self.pad_x
    }

    #[inline]
    pub fn left_content_w(&self) -> f32 {
        (self.left_w - (self.pane_content_inset() + self.pad_x) * 2.0).max(40.0)
    }

    #[inline]
    pub fn right_content_x(&self) -> f32 {
        self.right_x + self.pane_content_inset() + self.pad_x
    }

    #[inline]
    pub fn right_content_w(&self) -> f32 {
        (self.right_w - (self.pane_content_inset() + self.pad_x) * 2.0).max(40.0)
    }
}

pub fn chronicle_pane_layout(_w: f32, h: f32, panel: [f32; 4]) -> ChroniclePaneLayout {
    let [px, py, pw, ph] = panel;
    let margin = chronicle_panel_margin(h);
    let inner_x = px + margin;
    let inner_y = py + margin;
    let inner_w = (pw - margin * 2.0).max(80.0);
    let inner_h = (ph - margin * 2.0).max(60.0);
    let gutter = (h * rhythm::GUTTER).max(6.0);
    let left_w = ((inner_w - gutter) * LEFT_PANE_FRAC).max(80.0);
    let right_w = inner_w - gutter - left_w;
    let row_px = typography::size(rhythm::BODY, h);
    let run_row_h = rhythm::line_h(row_px) * 1.12 + h * rhythm::ROW_PAD * 0.55;
    let pad_x = (h * rhythm::PAD_X).max(8.0);
    let pad_y = (h * rhythm::PAD_Y * 0.72).max(6.0);
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
pub fn chronicle_panel_margin(h: f32) -> f32 {
    (h * rhythm::SHEET_INSET).max(4.0)
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

#[derive(Clone, Copy, Debug)]
struct ChronicleLayoutMetrics {
    body: f32,
    line_h: f32,
    title_h: f32,
    gap: f32,
    chart_h: f32,
    bar_row_h: f32,
    kpi_h: f32,
    footer_h: f32,
}

/// Vertical extent of the career/run-detail footer stat card (mirrors value + label rows).
fn chronicle_footer_band_height(h: f32) -> f32 {
    let cap_h = rhythm::line_h(typography::size(rhythm::CAPTION, h));
    // `push_career_pane` / `push_run_detail_pane`: value at fy+4 (cap_h+2), label at fy+cap_h+8.
    2.0 * cap_h + 12.0
}

fn layout_constants(h: f32) -> ChronicleLayoutMetrics {
    let body = typography::size(rhythm::BODY, h);
    let pane_title_px = typography::size(rhythm::PANE_TITLE, h);
    let line_h = rhythm::line_h(body);
    let title_h = rhythm::line_h(pane_title_px);
    let gap = rhythm::section_gap(h);
    let chart_h = (h * 0.082).max(64.0);
    let bar_row_h = rhythm::line_h(typography::size(rhythm::CAPTION, h));
    let cap_h = rhythm::line_h(typography::size(rhythm::CAPTION, h));
    let val_h = rhythm::line_h(body);
    let kpi_h = kpi_strip_height(h, cap_h, val_h);
    let footer_h = chronicle_footer_band_height(h);
    ChronicleLayoutMetrics {
        body,
        line_h,
        title_h,
        gap,
        chart_h,
        bar_row_h,
        kpi_h,
        footer_h,
    }
}

#[derive(Clone, Copy, Debug)]
struct ChronicleTypeScale {
    section_px: f32,
    body: f32,
    caption_px: f32,
    micro_px: f32,
    metrics: ChronicleLayoutMetrics,
}

struct ChronicleEmit<'a> {
    quads: &'a mut Vec<GpuInstance>,
    squircle_quads: &'a mut Vec<GpuInstance>,
    labels: &'a mut Vec<TextLabel>,
    images: &'a mut Vec<ImageQuad>,
    showcase_tiles: &'a mut Vec<ShowcaseTilePlacement>,
}

struct ChroniclePaneDraw<'a> {
    progress: &'a PlayerProgress,
    panes: ChroniclePaneLayout,
    scroll: f32,
    type_scale: ChronicleTypeScale,
    chronicle_last_seen_run_len: u32,
    emit: ChronicleEmit<'a>,
}

fn run_log_list_clip(panes: ChroniclePaneLayout) -> ChartClip {
    ChartClip {
        top: panes.content_y(),
        bottom: panes.content_y() + panes.content_h(),
    }
}

fn run_log_list_viewport_h(layout: ChroniclePaneLayout, title_h: f32, gap: f32) -> f32 {
    (layout.content_h() - title_h - gap * 0.25).max(1.0)
}

/// Compact run receipt columns (`#` + outcome · ordeal).
struct RunLogReceiptLayout {
    id_x: f32,
    id_w: f32,
    boss_x: f32,
    boss_w: f32,
    /// Single row font size shared by all receipt columns.
    row_font_px: f32,
}

const RUN_LOG_ID_SAMPLE: &str = "99 Loss";
/// Cap for the `#` + outcome column; ordeal name uses the remainder.
const RUN_LOG_ID_MAX_FRAC: f32 = 0.32;
const RUN_LOG_BOSS_MIN_W: f32 = 28.0;

fn run_log_receipt_layout(
    base_x: f32,
    pane_w: f32,
    row_inset: f32,
    target_font_px: f32,
    min_font_px: f32,
) -> RunLogReceiptLayout {
    let inner_x = base_x + row_inset;
    let inner_w = (pane_w - row_inset * 2.0).max(80.0);
    let col_gap = 6.0;
    let min_px = min_font_px.max(8.0);

    let mut row_font_px = target_font_px.max(min_px);
    let (id_w, boss_x, boss_w) = loop {
        let measured_id =
            chart_primitives::measure_text_width(RUN_LOG_ID_SAMPLE, row_font_px, true) + 6.0;
        let id_w = measured_id.min(inner_w * RUN_LOG_ID_MAX_FRAC);
        let boss_x = inner_x + id_w + col_gap;
        let boss_w = inner_x + inner_w - boss_x;
        if boss_w >= RUN_LOG_BOSS_MIN_W || row_font_px <= min_px {
            break (id_w, boss_x, boss_w);
        }
        row_font_px -= 0.5;
    };

    RunLogReceiptLayout {
        id_x: inner_x,
        id_w,
        boss_x,
        boss_w,
        row_font_px,
    }
}

fn run_log_row_text_layout(row_y: f32, row_h: f32, font_px: f32) -> (f32, f32) {
    let row_text_h = rhythm::line_h(font_px);
    let text_y = row_y + (row_h - row_text_h) * 0.5;
    (text_y, row_text_h)
}

struct RunLogListLayout {
    list_top: f32,
    header_band_h: f32,
}

fn run_log_list_layout(
    panes: ChroniclePaneLayout,
    title_h: f32,
    gap: f32,
    cap_h: f32,
    row_pad: f32,
    y_bias: f32,
) -> RunLogListLayout {
    RunLogListLayout {
        list_top: panes.content_y() + y_bias + title_h + gap * 0.25,
        header_band_h: cap_h + row_pad * 0.45,
    }
}

/// Vertical bias for the run-log block — only center the empty-state placeholder.
fn run_log_column_y_bias(
    panes: ChroniclePaneLayout,
    title_h: f32,
    gap: f32,
    entry_count: usize,
) -> f32 {
    if entry_count == 0 {
        let list_gap = gap * 0.25;
        let view_h = run_log_list_viewport_h(panes, title_h, gap);
        let col_used = title_h + list_gap + view_h;
        return (panes.content_h() - col_used).max(0.0) * 0.5;
    }
    0.0
}

fn run_log_row_y(
    layout: &RunLogListLayout,
    list_i: usize,
    run_row_h: f32,
    scroll: f32,
) -> (f32, f32) {
    if list_i == 0 {
        (layout.list_top + layout.header_band_h - scroll, run_row_h)
    } else {
        let runs_top = layout.list_top + layout.header_band_h + run_row_h;
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
    layout.header_band_h + entry_count as f32 * run_row_h
}

#[inline]
fn kpi_strip_height(h: f32, cap_h: f32, val_h: f32) -> f32 {
    let inset = rhythm::card_inset(h);
    let stack = rhythm::stack_gap(h);
    inset * 2.0 + cap_h + stack + val_h
}

const TILE_STRIP_GAP: f32 = 3.0;
const TILE_STRIP_ASPECT: f32 = 1.28;
const TILE_STRIP_MIN_W: f32 = 14.0;
const TILE_STRIP_MAX_W: f32 = 44.0;
const TILE_STRIP_MAX_COUNT: usize = 14;

fn tile_strip_height(strip_w: f32, tile_count: usize) -> f32 {
    if tile_count == 0 {
        return 0.0;
    }
    let n = tile_count.min(TILE_STRIP_MAX_COUNT) as f32;
    let tw = ((strip_w - TILE_STRIP_GAP * (n - 1.0)) / n).clamp(TILE_STRIP_MIN_W, TILE_STRIP_MAX_W);
    tw * TILE_STRIP_ASPECT
}

fn run_detail_hero_height(
    cap_h: f32,
    val_h: f32,
    strip_w: f32,
    tiles: &[crate::core::tile::Tile],
) -> f32 {
    let tile_h = tile_strip_height(strip_w, tiles.len());
    (8.0 + cap_h + 8.0 + val_h + 14.0 + tile_h + 10.0).max(48.0)
}

fn insights_band_height(h: f32, rw: f32, progress: &PlayerProgress, cap_h: f32, val_h: f32) -> f32 {
    let inset = rhythm::card_inset(h);
    let stack = rhythm::stack_gap(h);
    let micro_h = rhythm::line_h(typography::size(rhythm::MICRO, h));
    let inner_w = insights_signature_inner_w(rw, stack, inset);
    let title_w = (cap_h * 5.2).max(72.0).min(inner_w * 0.28);
    let tiles_w = (inner_w - title_w - stack).max(60.0);
    let tile_strip_h = archive_career::career_signature_record(progress)
        .map(|r| tile_strip_height(tiles_w, r.best_hand_tiles.len()))
        .unwrap_or(0.0);
    let title_row_h = cap_h.max(tile_strip_h);
    let text_stack = title_row_h + stack + val_h + stack * 0.35 + val_h * 0.92;
    let side_col = cap_h + stack + val_h.max(micro_h);
    inset * 2.0 + text_stack.max(side_col)
}

fn career_records_band_h(
    cap_h: f32,
    tight: f32,
    bar_row_h: f32,
    ordeal_rows: &[archive_career::OrdealRecordRow],
) -> f32 {
    let ordeal_block = cap_h + tight * 0.5 + ordeal_rows.len().min(5) as f32 * bar_row_h;
    let wing_block = career_ante_outcomes_block_h(cap_h, tight, bar_row_h);
    ordeal_block.max(wing_block) + tight * 0.35
}

fn career_tail_content_height(
    progress: &PlayerProgress,
    m: ChronicleLayoutMetrics,
    tight: f32,
    cap_h: f32,
) -> f32 {
    let mut tail = cap_h + tight * 0.5;
    let buckets = archive_career::score_distribution_buckets(progress);
    let dist_h = buckets.len() as f32 * m.bar_row_h;
    tail += dist_h + tight;
    tail += cap_h + tight * 0.5;
    let yaku_n = archive_career::career_top_yaku(progress, 6).len().min(6);
    tail += yaku_n as f32 * m.bar_row_h + tight;
    tail
}

fn career_ante_outcomes_block_h(cap_h: f32, tight: f32, bar_row_h: f32) -> f32 {
    cap_h + tight * 0.5 + bar_row_h * 2.2
}

fn career_score_history_band_h(window_h: f32, cap_h: f32) -> f32 {
    let axis_label_h = cap_h + 2.0;
    let plot_h = (window_h * 0.085).clamp(68.0, 96.0);
    plot_h + axis_label_h
}

/// Total vertical extent of the career pane document (must mirror [`push_career_pane`]).
fn career_content_height(
    w: f32,
    h: f32,
    panel: [f32; 4],
    progress: &PlayerProgress,
    m: ChronicleLayoutMetrics,
) -> f32 {
    let panes = chronicle_pane_layout(w, h, panel);
    let rw = panes.right_content_w();
    let cap_h = rhythm::line_h(typography::size(rhythm::CAPTION, h));
    let val_h = rhythm::line_h(m.body);
    let runs = serious_runs_chronological(progress);
    let mut doc_y = 0.0_f32;

    doc_y += m.title_h + m.gap * 0.35;
    doc_y += m.kpi_h + m.gap * 0.4;

    if archive_career::career_signature_record(progress).is_some()
        || !archive_career::career_tiles(progress).is_empty()
    {
        doc_y += insights_band_height(h, rw, progress, cap_h, val_h) + m.gap * 0.4;
    }

    if runs.is_empty() {
        return doc_y;
    }

    let tight = m.gap * 0.4;
    doc_y += cap_h + tight * 0.5;
    let ordeal_rows = archive_career::career_ordeal_records(progress);
    doc_y += career_score_history_band_h(h, cap_h) + tight;
    doc_y += career_records_band_h(cap_h, tight, m.bar_row_h, &ordeal_rows) + tight;
    doc_y += career_tail_content_height(progress, m, tight, cap_h);

    doc_y
}

/// Total vertical extent of the run-detail pane (must mirror [`push_run_detail_pane`]).
fn run_detail_content_height(
    model: &archive_career::RunDetailModel,
    rec: &RunRecord,
    m: ChronicleLayoutMetrics,
    cap_h: f32,
    val_h: f32,
    strip_w: f32,
) -> f32 {
    let mut doc_y = 0.0_f32;

    doc_y += m.title_h + 4.0;
    doc_y += cap_h + m.gap * 0.75;
    doc_y += run_detail_hero_height(cap_h, val_h, strip_w, &model.tiles) + m.gap;

    if !model.yaku_rows.is_empty() {
        doc_y += m.title_h + m.gap * 0.4;
        doc_y += model.yaku_rows.len().min(8) as f32 * (m.bar_row_h + 3.0);
        doc_y += m.gap * 0.5;
    }

    doc_y += m.title_h + m.gap * 0.35;
    doc_y += model.score_lines.len() as f32 * m.line_h;
    doc_y += m.gap * 0.5;

    if model.wing_scores.len() >= 2 {
        doc_y += m.title_h + m.gap * 0.4;
        doc_y += m.chart_h + m.gap;
    }

    if !model.timeline.is_empty() {
        doc_y += m.title_h + m.gap * 0.35;
        doc_y += model.timeline.len() as f32 * m.line_h * 0.9;
        doc_y += m.gap * 0.5;
    }

    if !rec.relics_owned.is_empty() {
        doc_y += m.title_h + m.gap * 0.35;
        doc_y += 36.0 + m.gap * 0.5;
    }

    if !rec.consumables_owned.is_empty() {
        doc_y += m.line_h + m.gap * 0.5;
    }

    if rec.memorial_kind.is_some() {
        doc_y += m.line_h + m.gap;
    }

    doc_y += m.footer_h;
    doc_y
}

pub fn chronicle_run_log_scroll_max(w: f32, h: f32, panel: [f32; 4], entry_count: usize) -> f32 {
    let panes = chronicle_pane_layout(w, h, panel);
    let m = layout_constants(h);
    let cap_h = rhythm::line_h(typography::size(rhythm::CAPTION, h));
    let row_pad = (h * rhythm::ROW_PAD).max(6.0);
    let list_layout = run_log_list_layout(panes, m.title_h, m.gap, cap_h, row_pad, 0.0);
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
    let caption_px = typography::size(rhythm::CAPTION, h);
    let cap_h = (caption_px / 0.55).ceil();
    let val_h = (m.body / 0.55).ceil();
    let strip_w = panes.right_content_w() - 20.0;
    let content = run_detail_content_height(&model, rec, m, cap_h, val_h, strip_w);
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
    let content = career_content_height(w, h, panel, progress, m);
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
    let pane_h = layout.inner_h + layout.margin * 2.0;
    let cap_h = rhythm::line_h(typography::size(rhythm::CAPTION, pane_h));
    let row_pad = (pane_h * rhythm::ROW_PAD).max(6.0);
    let list_layout = run_log_list_layout(layout, title_h, gap, cap_h, row_pad, 0.0);
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
    rhythm::line_h(typography::size(
        rhythm::PANE_TITLE,
        layout.inner_h + layout.margin * 2.0,
    ))
}

#[inline]
fn gap_for_layout(layout: ChroniclePaneLayout) -> f32 {
    ((layout.inner_h + layout.margin * 2.0) * rhythm::SECTION_GAP).max(12.0)
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
    let cap_h = rhythm::line_h(typography::size(rhythm::CAPTION, h));
    let row_pad = (h * rhythm::ROW_PAD).max(6.0);
    let y_bias = run_log_column_y_bias(panes, m.title_h, m.gap, entry_count);
    let list_layout = run_log_list_layout(panes, m.title_h, m.gap, cap_h, row_pad, y_bias);
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

fn push_chronicle_yaku_pill(
    emit: &mut ChronicleEmit<'_>,
    clip: ChartClip,
    x: f32,
    y: f32,
    row_h: f32,
    name: &str,
    max_w: f32,
    caption_px: f32,
) -> f32 {
    push_yaku_pill(
        emit.squircle_quads,
        emit.labels,
        clip,
        x,
        y,
        row_h,
        name,
        max_w,
        archive_career::yaku_pill_face(),
        archive_career::yaku_pill_ink(),
        archive_career::yaku_pill_rim(),
        caption_px,
    )
}

fn push_label_clipped(out: &mut Vec<TextLabel>, rect: [f32; 4], clip: ChartClip, label: TextLabel) {
    chart_primitives::push_label_clipped(out, rect, clip, label);
}

fn push_quad_clipped(out: &mut Vec<GpuInstance>, rect: [f32; 4], clip: ChartClip, c: [f32; 4]) {
    chart_quad(out, rect, clip, c);
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
    let n = tiles.len().min(TILE_STRIP_MAX_COUNT) as f32;
    let tw = ((w - TILE_STRIP_GAP * (n - 1.0)) / n).clamp(TILE_STRIP_MIN_W, TILE_STRIP_MAX_W);
    let th = tw * TILE_STRIP_ASPECT;
    let tile_size = tw * 0.92;
    let center_y = y + th * 0.5;
    for (i, tile) in tiles.iter().take(TILE_STRIP_MAX_COUNT).enumerate() {
        let tx = x + i as f32 * (tw + TILE_STRIP_GAP);
        if y + th > clip.bottom || y + th < clip.top {
            continue;
        }
        emit.showcase_tiles.push(ShowcaseTilePlacement {
            tile: *tile,
            center_pos: [tx + tw * 0.5, center_y, 0.0],
            rotation: TOP_DOWN_TILE_ROTATION,
            scale: 1.0,
            size_px: tile_size,
            brightness: 1.0,
            opacity: 1.0,
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
            outline_sel: None,
            pick_id: None,
            overlay_rect_group: None,
        });
    }
}

fn push_relic_row(
    emit: &mut ChronicleEmit<'_>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    relics: &[crate::core::relic::RelicId],
) {
    let icon = 28.0_f32;
    let gap = 6.0;
    let scissor = [x, clip.top, w.max(1.0), (clip.bottom - clip.top).max(0.0)];
    if scissor[3] <= 0.0 || y >= clip.bottom || y + icon <= clip.top {
        return;
    }
    for (i, rid) in relics.iter().take(10).enumerate() {
        let ix = x + i as f32 * (icon + gap);
        if intersect_rect([ix, y, icon, icon], scissor).is_none() {
            continue;
        }
        emit.images.push(ImageQuad {
            inst: GpuInstance {
                rect: [ix, y, icon, icon],
                color: [1.0, 1.0, 1.0, 0.96],
                user: 0,
            },
            source: ImageQuadSource::Relic(*rid),
            clip_rect: Some(scissor),
        });
    }
}

fn push_run_log(draw: ChroniclePaneDraw<'_>, focused: Option<usize>) {
    let ChroniclePaneDraw {
        progress,
        panes,
        scroll,
        type_scale,
        chronicle_last_seen_run_len,
        emit,
    } = draw;
    let ChronicleTypeScale {
        section_px,
        body,
        caption_px,
        micro_px,
        metrics,
        ..
    } = type_scale;
    let ChronicleLayoutMetrics { title_h, gap, .. } = metrics;
    let indices = archive_career::chronicle_indices_recent_first(progress);
    let run_count = indices.len();
    let entry_count = archive_career::chronicle_list_entry_count(progress);
    let cap_h = rhythm::line_h(caption_px);
    let list_clip = run_log_list_clip(panes);
    let clip = list_clip;

    let row_pad = (panes.inner_h + panes.margin * 2.0) * rhythm::ROW_PAD;
    let row_pad = row_pad.max(6.0);
    let inset = panes.pad_x * 0.65;
    let row_inset = inset;
    let y_bias = run_log_column_y_bias(panes, title_h, gap, entry_count);
    let title_y = panes.content_y() + y_bias;
    let title_w = panes.left_content_w();
    push_label_clipped(
        emit.labels,
        [panes.left_content_x(), title_y, title_w * 0.42, title_h],
        list_clip,
        TextLabel {
            rect: [panes.left_content_x(), title_y, title_w * 0.42, title_h],
            text: "Run log".into(),
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    push_label_clipped(
        emit.labels,
        [
            panes.left_content_x() + title_w * 0.38,
            title_y,
            title_w * 0.62 - row_inset,
            title_h,
        ],
        list_clip,
        TextLabel {
            rect: [
                panes.left_content_x() + title_w * 0.38,
                title_y,
                title_w * 0.62 - row_inset,
                title_h,
            ],
            text: format!("{run_count} runs"),
            color: color::alpha(color::STONE, 0.88),
            font_px: Some(caption_px),
            align: TextAlign::Right,
            mono: true,
            ..Default::default()
        },
    );

    let list_layout = run_log_list_layout(panes, title_h, gap, cap_h, row_pad, y_bias);
    let header_y = list_layout.list_top - scroll;
    let pane_h = panes.inner_h + panes.margin * 2.0;
    let min_font_px = typography::readable_floor_px(pane_h);
    let receipt = run_log_receipt_layout(
        panes.left_content_x(),
        panes.left_content_w(),
        row_inset,
        body,
        min_font_px,
    );
    let hdr = color::alpha(color::STONE, 0.72);
    for (hx, hw, label, align) in [
        (receipt.id_x, receipt.id_w, "#", TextAlign::Left),
        (receipt.boss_x, receipt.boss_w, "Ordeal", TextAlign::Left),
    ] {
        push_label_clipped(
            emit.labels,
            [hx, header_y, hw, cap_h],
            clip,
            tabular(TextLabel {
                rect: [hx, header_y, hw, cap_h],
                text: label.into(),
                color: hdr,
                font_px: Some(caption_px),
                align,
                ..Default::default()
            }),
        );
    }

    let runs_top = list_layout.list_top + list_layout.header_band_h + panes.run_row_h;
    chart_primitives::push_quad(
        emit.quads,
        [
            panes.left_content_x(),
            runs_top - scroll - 1.0,
            panes.left_content_w(),
            1.0,
        ],
        color::alpha(color::BRASS, 0.2),
    );

    if run_count == 0 {
        let row_y = list_layout.list_top + list_layout.header_band_h - scroll;
        push_label_clipped(
            emit.labels,
            [
                panes.left_content_x(),
                row_y,
                panes.left_content_w(),
                panes.run_row_h * 2.0,
            ],
            clip,
            TextLabel {
                rect: [
                    panes.left_content_x(),
                    row_y,
                    panes.left_content_w(),
                    panes.run_row_h * 2.0,
                ],
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
                list_clip.top,
                panes.left_content_w(),
                list_clip.bottom - list_clip.top,
            ],
        )
        .is_none()
        {
            continue;
        }
        let selected = focused == Some(list_i);
        let row_font_px = receipt.row_font_px;
        let (text_y, row_text_h) = run_log_row_text_layout(row_y, row_h, row_font_px);

        if selected {
            push_quad_clipped(
                emit.quads,
                [panes.left_content_x(), row_y, panes.left_content_w(), row_h],
                clip,
                color::alpha(color::WALNUT_RAISED, 0.88),
            );
            push_quad_clipped(
                emit.quads,
                [panes.left_content_x(), row_y, 3.0, row_h],
                clip,
                color::alpha(color::CHAMPAGNE, 0.75),
            );
        } else if list_i > 0 {
            chart_primitives::push_quad_clipped(
                emit.quads,
                [
                    panes.left_content_x() + row_inset,
                    row_y + row_h - 1.0,
                    panes.left_content_w() - row_inset * 2.0,
                    1.0,
                ],
                clip,
                color::alpha(color::BRASS, 0.14),
            );
        }

        if list_i == 0 {
            push_label_clipped(
                emit.labels,
                [
                    receipt.id_x,
                    text_y,
                    panes.left_content_w() - row_inset * 2.0,
                    row_text_h,
                ],
                clip,
                TextLabel {
                    rect: [
                        receipt.id_x,
                        text_y,
                        panes.left_content_w() - row_inset * 2.0,
                        row_text_h,
                    ],
                    text: "Summary".into(),
                    color: if selected {
                        color::CHAMPAGNE
                    } else {
                        color::alpha(color::PARCHMENT, 0.96)
                    },
                    font_px: Some(row_font_px),
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
        let run_is_new =
            crate::core::archive_seen::chronicle_run_is_new(hist_idx, chronicle_last_seen_run_len);
        let id_line = format!(
            "{display:>2} {}",
            archive_career::chronicle_run_outcome_short(rec)
        );
        push_label_clipped(
            emit.labels,
            [receipt.id_x, text_y, receipt.id_w, row_text_h],
            clip,
            tabular(TextLabel {
                rect: [receipt.id_x, text_y, receipt.id_w, row_text_h],
                text: id_line,
                color: if selected {
                    color::CHAMPAGNE
                } else {
                    outcome_color
                },
                font_px: Some(row_font_px),
                align: TextAlign::Left,
                ..Default::default()
            }),
        );
        if let Some(boss) = archive_career::chronicle_run_log_ordeal_line(rec) {
            let mut boss_x = receipt.boss_x;
            let mut boss_w = receipt.boss_w;
            let boss = chart_primitives::truncate_text_to_width(
                &boss,
                boss_w.max(1.0),
                row_font_px,
                false,
            );
            if run_is_new {
                let stamp = (row_font_px * 0.95).max(10.0);
                chronicle_charts::push_discovery_stamp(
                    emit.quads,
                    emit.labels,
                    clip,
                    boss_x,
                    text_y + (row_text_h - stamp) * 0.5,
                    stamp,
                    micro_px,
                );
                boss_x += stamp + 2.0;
                boss_w -= stamp + 2.0;
            }
            push_label_clipped(
                emit.labels,
                [boss_x, text_y, boss_w.max(1.0), row_text_h],
                clip,
                TextLabel {
                    rect: [boss_x, text_y, boss_w.max(1.0), row_text_h],
                    text: boss,
                    color: color::alpha(color::PARCHMENT, 0.92),
                    font_px: Some(row_font_px),
                    align: TextAlign::Left,
                    ..Default::default()
                },
            );
        }
    }
}

fn push_insight_column(
    emit: &mut ChronicleEmit<'_>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    _band_h: f32,
    inset: f32,
    stack: f32,
    cap_h: f32,
    val_h: f32,
    caption_px: f32,
    body_px: f32,
    micro_px: f32,
    tile: &archive_career::CareerTile,
) {
    let text_w = (w - inset * 2.0).max(1.0);
    let mut ly = y + inset;
    push_label_clipped(
        emit.labels,
        [x + inset, ly, text_w, cap_h],
        clip,
        TextLabel {
            rect: [x + inset, ly, text_w, cap_h],
            text: tile.label.into(),
            color: color::STONE,
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    ly += cap_h + stack;

    if let Some(yk) = tile.yaku {
        let pill_row_h = val_h.max(caption_px * 1.15);
        push_chronicle_yaku_pill(
            emit,
            clip,
            x + inset,
            ly,
            pill_row_h,
            yk.name(),
            text_w,
            caption_px,
        );
        ly += pill_row_h + stack * 0.5;
        if let Some(d) = &tile.detail {
            let micro_h = rhythm::line_h(micro_px);
            push_label_clipped(
                emit.labels,
                [x + inset, ly, text_w, micro_h],
                clip,
                TextLabel {
                    rect: [x + inset, ly, text_w, micro_h],
                    text: d.clone(),
                    color: color::alpha(color::STONE, 0.9),
                    font_px: Some(micro_px),
                    align: TextAlign::Left,
                    ..Default::default()
                },
            );
        }
        return;
    }

    let line = if let Some(d) = &tile.detail {
        if tile.value.is_empty() {
            d.clone()
        } else {
            format!("{} · {}", tile.value, d)
        }
    } else {
        tile.value.clone()
    };
    push_colored_label_clipped(
        emit.labels,
        [x + inset, ly, text_w, val_h],
        clip,
        &line,
        color::CHAMPAGNE,
        body_px,
        TextAlign::Left,
        false,
    );
}

fn push_career_insights_band(
    emit: &mut ChronicleEmit<'_>,
    clip: ChartClip,
    h: f32,
    doc_y: f32,
    rx: f32,
    rw: f32,
    progress: &PlayerProgress,
    scroll: f32,
    inset: f32,
    stack: f32,
    cap_h: f32,
    val_h: f32,
    caption_px: f32,
    body_px: f32,
    micro_px: f32,
) {
    let has_sig = archive_career::career_signature_record(progress).is_some();
    let highlights: Vec<_> = archive_career::career_tiles(progress);
    if !has_sig && highlights.is_empty() {
        return;
    }
    let band_h = insights_band_height(h, rw, progress, cap_h, val_h);
    let by = doc_y - scroll;
    let cols = kpi_column_layout(rw, stack);
    let (sig_x, sig_w) = kpi_span_rect(rx, cols, 0, 2);
    let (fav_x, fav_w) = kpi_column_rect(rx, cols, 2);
    let (nem_x, nem_w) = kpi_column_rect(rx, cols, 3);

    for &(bx, bw) in &[(sig_x, sig_w), (fav_x, fav_w), (nem_x, nem_w)] {
        push_ledger_panel_clipped(
            emit.quads,
            clip,
            [bx, by, bw, band_h],
            LedgerPanelStyle::INSIGHT,
        );
    }

    let col_gap = stack;
    let inner_w = (sig_w - inset * 2.0).max(60.0);
    let title_w = (cap_h * 5.2).max(72.0).min(inner_w * 0.28);
    let tiles_w = (inner_w - title_w - col_gap).max(60.0);

    if let Some(rec) = archive_career::career_signature_record(progress) {
        let tx = sig_x + inset;
        let header_y = by + inset;
        let tile_strip_h = tile_strip_height(tiles_w, rec.best_hand_tiles.len());
        let title_row_h = cap_h.max(tile_strip_h);
        push_label_clipped(
            emit.labels,
            [tx, header_y, title_w, cap_h],
            clip,
            TextLabel {
                rect: [tx, header_y, title_w, cap_h],
                text: "Signature hand".into(),
                color: color::STONE,
                font_px: Some(caption_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
        if tile_strip_h > 0.0 {
            let tiles_x = tx + title_w + col_gap;
            let tiles_y = header_y + (title_row_h - tile_strip_h) * 0.5;
            push_tile_strip(emit, clip, tiles_x, tiles_y, tiles_w, &rec.best_hand_tiles);
        }
        let stats = archive_career::signature_hand_stats(progress, rec);
        let summary_y = header_y + title_row_h + stack;
        let mut tag_x = tx;
        for yk in stats.yaku_tags {
            let pill_w = push_chronicle_yaku_pill(
                emit,
                clip,
                tag_x,
                summary_y,
                val_h * 0.92,
                yk.name(),
                inner_w * 0.24,
                micro_px,
            );
            tag_x += pill_w + 4.0;
            if tag_x > tx + inner_w - 40.0 {
                break;
            }
        }
        let meta_y = summary_y + val_h + stack * 0.35;
        let meta = format!(
            "{} · avg {} · {}×",
            archive_career::format_chips_compact(rec.best_structure_score),
            archive_career::format_chips_compact(stats.avg_score),
            stats.times_used
        );
        push_colored_label_clipped(
            emit.labels,
            [tx, meta_y, inner_w, val_h * 0.92],
            clip,
            &meta,
            color::CHAMPAGNE,
            caption_px,
            TextAlign::Left,
            false,
        );
    } else if let Some(tile) = highlights.first() {
        push_insight_column(
            emit, clip, sig_x, by, sig_w, band_h, inset, stack, cap_h, val_h, caption_px, body_px,
            micro_px, tile,
        );
    }

    let side_tiles = if has_sig {
        highlights.iter().take(2).collect::<Vec<_>>()
    } else {
        highlights.iter().skip(1).take(2).collect::<Vec<_>>()
    };
    if let Some(tile) = side_tiles.first() {
        push_insight_column(
            emit, clip, fav_x, by, fav_w, band_h, inset, stack, cap_h, val_h, caption_px, body_px,
            micro_px, tile,
        );
    }
    if let Some(tile) = side_tiles.get(1) {
        push_insight_column(
            emit, clip, nem_x, by, nem_w, band_h, inset, stack, cap_h, val_h, caption_px, body_px,
            micro_px, tile,
        );
    }
}

fn push_dense_section_title(
    labels: &mut Vec<TextLabel>,
    clip: ChartClip,
    x: f32,
    y: f32,
    w: f32,
    cap_h: f32,
    text: &str,
    caption_px: f32,
) {
    push_label_clipped(
        labels,
        [x, y, w, cap_h],
        clip,
        TextLabel {
            rect: [x, y, w, cap_h],
            text: text.into(),
            color: color::alpha(color::GOLD, 0.92),
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
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
        chronicle_last_seen_run_len: _,
        mut emit,
    } = draw;
    let ChronicleTypeScale {
        section_px,
        body,
        caption_px,
        micro_px: _,
        metrics,
        ..
    } = type_scale;
    let clip = pane_clip(panes);
    let cap_h = rhythm::line_h(caption_px);
    let val_h = rhythm::line_h(body);
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
        "Career ledger",
        section_px,
    );
    doc_y += metrics.title_h + metrics.gap * 0.35;

    let micro_px = type_scale.micro_px;
    let inset = rhythm::card_inset(h);
    let stack = rhythm::stack_gap(h);
    let tight = metrics.gap * 0.4;

    let kpis = archive_career::career_kpi_strip(progress);
    let kpi_cols = kpi_column_layout(rw, stack);
    let kpi_h = metrics.kpi_h;
    for (i, kpi) in kpis.iter().take(KPI_COUNT).enumerate() {
        let (kx, kpi_w) = kpi_column_rect(rx, kpi_cols, i);
        let ky = ry(doc_y);
        chronicle_charts::push_kpi_card(
            emit.quads,
            emit.labels,
            clip,
            kx,
            ky,
            kpi_w,
            kpi_h,
            inset,
            stack,
            cap_h,
            val_h,
            caption_px,
            body,
            kpi,
        );
    }
    doc_y += kpi_h + tight;

    let insights_h = insights_band_height(h, rw, progress, cap_h, val_h);
    push_career_insights_band(
        &mut emit,
        clip,
        h,
        panes.content_y() + doc_y,
        rx,
        rw,
        progress,
        scroll,
        inset,
        stack,
        cap_h,
        val_h,
        caption_px,
        body,
        micro_px,
    );
    doc_y += insights_h + tight;

    if runs.is_empty() {
        return;
    }

    let label_w = (rw * 0.34).min(160.0);
    let history_points = archive_career::career_score_history_points(progress);
    let avg_score = archive_career::career_average_score(progress);
    let personal_best = archive_career::max_total_score_serious(progress).unwrap_or(0);
    let ordeal_rows = archive_career::career_ordeal_records(progress);

    let title_y = ry(doc_y);
    push_dense_section_title(
        emit.labels,
        clip,
        rx,
        title_y,
        rw * 0.42,
        cap_h,
        "Score history",
        caption_px,
    );
    let callout = if avg_score > 0 {
        format!(
            "Avg {} · Peak {}",
            archive_career::format_chips_compact(avg_score),
            archive_career::format_chips_compact(personal_best)
        )
    } else {
        format!(
            "Peak {}",
            archive_career::format_chips_compact(personal_best)
        )
    };
    push_label_clipped(
        emit.labels,
        [rx + rw * 0.38, title_y, rw * 0.62, cap_h],
        clip,
        tabular(TextLabel {
            rect: [rx + rw * 0.38, title_y, rw * 0.62, cap_h],
            text: callout,
            color: color::alpha(color::STONE, 0.88),
            font_px: Some(typography::size(rhythm::MICRO, h)),
            align: TextAlign::Right,
            ..Default::default()
        }),
    );
    doc_y += cap_h + tight * 0.5;
    let score_history_h = career_score_history_band_h(h, cap_h);
    let chart_top = ry(doc_y);
    chronicle_charts::push_score_history_ledger(
        emit.quads,
        emit.labels,
        clip,
        rx,
        chart_top,
        rw,
        score_history_h,
        &history_points,
        avg_score,
        personal_best,
        caption_px,
        body,
        micro_px,
        true,
        false,
    );
    doc_y += score_history_h + tight * 0.35;

    let records_y = ry(doc_y);
    let records_h = career_records_band_h(cap_h, tight, metrics.bar_row_h, &ordeal_rows);
    let records_gap = stack;
    let half_w = ((rw - records_gap) * 0.5).max(120.0);
    push_dense_section_title(
        emit.labels,
        clip,
        rx,
        records_y,
        half_w,
        cap_h,
        "Ordeal record",
        caption_px,
    );
    push_dense_section_title(
        emit.labels,
        clip,
        rx + half_w + records_gap,
        records_y,
        half_w,
        cap_h,
        "Wing outcomes",
        caption_px,
    );
    let rows_y = records_y + cap_h + tight * 0.5;
    chronicle_charts::push_ordeal_record_rows(
        emit.labels,
        emit.quads,
        clip,
        rx,
        rows_y,
        half_w,
        metrics.bar_row_h,
        &ordeal_rows,
        caption_px,
        micro_px,
        body,
    );
    let ante_cells = archive_career::career_ante_outcome_matrix(progress);
    chronicle_charts::push_ante_outcome_matrix(
        emit.quads,
        emit.labels,
        clip,
        rx + half_w + records_gap,
        rows_y,
        half_w,
        metrics.bar_row_h * 2.2,
        &ante_cells,
        caption_px,
        micro_px,
    );
    doc_y += records_h + tight * 0.35;

    push_dense_section_title(
        emit.labels,
        clip,
        rx,
        ry(doc_y),
        rw,
        cap_h,
        "Score distribution",
        caption_px,
    );
    doc_y += cap_h + tight * 0.5;
    let buckets = archive_career::score_distribution_buckets(progress);
    let max_b = buckets.iter().map(|b| b.count).max().unwrap_or(1).max(1);
    let total_runs = runs.len().max(1) as f32;
    for (i, b) in buckets.iter().enumerate() {
        let row_top = ry(doc_y) + i as f32 * metrics.bar_row_h;
        let pct = (b.count as f32 / total_runs * 100.0).round() as u32;
        chronicle_charts::push_tile_bucket_hbar(
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
            pct,
            label_w,
            caption_px,
            body,
        );
    }
    let dist_h = buckets.len() as f32 * metrics.bar_row_h;
    doc_y += dist_h + tight * 0.35;

    let yaku_rows = archive_career::career_top_yaku(progress, 6);
    let max_y = yaku_rows.first().map(|(_, c)| *c).unwrap_or(1).max(1);
    push_dense_section_title(
        emit.labels,
        clip,
        rx,
        ry(doc_y),
        rw,
        cap_h,
        "Yaku fingerprint",
        caption_px,
    );
    doc_y += cap_h + tight * 0.5;
    chronicle_charts::push_yaku_fingerprint_rows(
        emit.squircle_quads,
        emit.quads,
        emit.labels,
        clip,
        rx,
        ry(doc_y),
        rw,
        metrics.bar_row_h,
        &yaku_rows,
        max_y,
        label_w,
        caption_px,
        body,
    );
}

fn push_run_detail_pane(draw: ChroniclePaneDraw<'_>, list_index: usize) {
    let ChroniclePaneDraw {
        progress,
        panes,
        scroll,
        type_scale,
        chronicle_last_seen_run_len: _,
        mut emit,
    } = draw;
    let ChronicleTypeScale {
        section_px,
        body,
        caption_px,
        micro_px,
        metrics,
        ..
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

    let outcome_color = archive_career::chronicle_run_outcome_color(rec);

    push_label_clipped(
        emit.labels,
        [rx, ry(doc_y), rw, metrics.title_h],
        clip,
        TextLabel {
            rect: [rx, ry(doc_y), rw, metrics.title_h],
            text: model.title_pins.clone(),
            color: color::GOLD,
            font_px: Some(section_px),
            align: TextAlign::Left,
            ..Default::default()
        },
    );
    if !model.title_rest.is_empty() {
        let pins_w = chart_primitives::pill_label_width(&model.title_pins, section_px);
        push_label_clipped(
            emit.labels,
            [
                rx + pins_w,
                ry(doc_y),
                (rw - pins_w).max(1.0),
                metrics.title_h,
            ],
            clip,
            TextLabel {
                rect: [
                    rx + pins_w,
                    ry(doc_y),
                    (rw - pins_w).max(1.0),
                    metrics.title_h,
                ],
                text: model.title_rest.clone(),
                color: outcome_color,
                font_px: Some(section_px),
                align: TextAlign::Left,
                ..Default::default()
            },
        );
    }
    doc_y += metrics.title_h + 4.0;
    push_label_clipped(
        emit.labels,
        [rx, ry(doc_y), rw, cap_h],
        clip,
        tabular(TextLabel {
            rect: [rx, ry(doc_y), rw, cap_h],
            text: model.timestamp_line.clone(),
            color: color::alpha(color::STONE, 0.9),
            font_px: Some(caption_px),
            align: TextAlign::Left,
            ..Default::default()
        }),
    );
    doc_y += cap_h + metrics.gap * 0.75;

    let strip_w = rw - 20.0;
    let hero_h = run_detail_hero_height(cap_h, val_h, strip_w, &model.tiles);
    let hy = ry(doc_y);
    push_ledger_panel_clipped(
        emit.quads,
        clip,
        [rx, hy, rw, hero_h],
        LedgerPanelStyle::SECTION,
    );
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
    push_colored_label_clipped(
        emit.labels,
        [rx + 10.0, hy + cap_h + 8.0, rw - 16.0, val_h],
        clip,
        &format!(
            "{} · {}",
            model.signature_name,
            archive_career::format_chips(model.signature_score)
        ),
        color::PARCHMENT,
        body,
        TextAlign::Left,
        true,
    );
    push_tile_strip(
        &mut emit,
        clip,
        rx + 10.0,
        hy + cap_h + val_h + 14.0,
        strip_w,
        &model.tiles,
    );
    doc_y += hero_h + metrics.gap;

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
            push_yaku_hbar_row(
                emit.squircle_quads,
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
                archive_career::yaku_pill_face(),
                archive_career::yaku_pill_ink(),
                archive_career::yaku_pill_rim(),
                color::alpha(color::chart::FILL, 0.82),
                color::alpha(color::PARCHMENT, 0.94),
                caption_px,
                body,
                Some(model.yaku_value_suffix),
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
        push_colored_label_clipped(
            emit.labels,
            [rx, ry(doc_y), rw, metrics.line_h],
            clip,
            line,
            color::alpha(color::PARCHMENT, 0.94),
            body,
            TextAlign::Left,
            true,
        );
        doc_y += metrics.line_h;
    }
    doc_y += metrics.gap * 0.5;

    if model.wing_scores.len() >= 2 {
        push_section_title(
            emit.labels,
            clip,
            rx,
            ry(doc_y),
            rw,
            metrics.title_h,
            "Wing progression",
            section_px,
        );
        doc_y += metrics.title_h + metrics.gap * 0.4;
        let chart_top = ry(doc_y);
        let max_s = model
            .wing_scores
            .iter()
            .map(|(_, s)| *s)
            .max()
            .unwrap_or(1)
            .max(1);
        let samples: Vec<f32> = model
            .wing_scores
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
            push_colored_label_clipped(
                emit.labels,
                [rx, ry(doc_y), rw, metrics.line_h * 0.9],
                clip,
                &format!("Wing {ante} · {blind} · {note}"),
                color::alpha(color::STONE, 0.92),
                caption_px,
                TextAlign::Left,
                true,
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
        push_relic_row(&mut emit, clip, rx, ry(doc_y), rw, &rec.relics_owned);
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
    push_ledger_panel_clipped(
        emit.quads,
        clip,
        [rx, fy, rw, metrics.footer_h],
        LedgerPanelStyle::SECTION,
    );
    let n = model.footer.len().max(1) as f32;
    let slot = rw / n;
    for (i, (label, value)) in model.footer.iter().enumerate() {
        let sx = rx + i as f32 * slot;
        let value_rect = [sx, fy + 4.0, slot, cap_h + 2.0];
        push_colored_label_clipped(
            emit.labels,
            value_rect,
            clip,
            value,
            archive_career::chronicle_footer_value_color(label),
            caption_px,
            TextAlign::Center,
            true,
        );
        push_label_clipped(
            emit.labels,
            [sx, fy + cap_h + 8.0, slot, cap_h],
            clip,
            TextLabel {
                rect: [sx, fy + cap_h + 8.0, slot, cap_h],
                text: label.clone(),
                color: color::alpha(color::STONE, 0.85),
                font_px: Some(micro_px),
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
    chronicle_last_seen_run_len: u32,
    out_quads: &mut Vec<GpuInstance>,
    out_squircle_quads: &mut Vec<GpuInstance>,
    out_labels: &mut Vec<TextLabel>,
    out_images: &mut Vec<ImageQuad>,
    out_showcase_tiles: &mut Vec<ShowcaseTilePlacement>,
) {
    let panes = chronicle_pane_layout(w, h, panel);
    let metrics = layout_constants(h);
    let type_scale = ChronicleTypeScale {
        section_px: typography::size(rhythm::PANE_TITLE, h),
        body: metrics.body,
        caption_px: typography::size(rhythm::CAPTION, h),
        micro_px: typography::size(rhythm::MICRO, h),
        metrics,
    };

    push_ledger_sheet(
        out_quads,
        w,
        h,
        panes.inner_x,
        panes.inner_y,
        panes.inner_w,
        panes.inner_h,
    );
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
        squircle_quads: out_squircle_quads,
        labels: out_labels,
        images: out_images,
        showcase_tiles: out_showcase_tiles,
    };

    push_run_log(
        ChroniclePaneDraw {
            progress,
            panes,
            scroll: view.run_log_scroll,
            type_scale,
            chronicle_last_seen_run_len,
            emit: ChronicleEmit {
                quads: emit.quads,
                squircle_quads: emit.squircle_quads,
                labels: emit.labels,
                images: emit.images,
                showcase_tiles: emit.showcase_tiles,
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
                chronicle_last_seen_run_len,
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
                chronicle_last_seen_run_len,
                emit,
            },
            list_i,
        );
    }
}

fn push_ledger_sheet(
    out: &mut Vec<GpuInstance>,
    window_w: f32,
    window_h: f32,
    inner_x: f32,
    inner_y: f32,
    inner_w: f32,
    inner_h: f32,
) {
    let b = metrics::tooltip_border_px(window_w, window_h);
    push_tooltip_frame_quads(out, inner_x, inner_y, inner_w, inner_h, b);
    chart_primitives::push_quad(
        out,
        [inner_x, inner_y, inner_w, inner_h * 0.18],
        color::alpha(color::WALNUT_INK, 0.22),
    );
}

pub fn chronicle_dim_gradient(panel: [f32; 4]) -> GradientQuadInstance {
    GradientQuadInstance {
        rect: panel,
        color: color::alpha(color::WALNUT_INK, 0.96),
        feather: [0.04, 0.0, 0.0, 0.0],
    }
}
