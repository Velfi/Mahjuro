//! Wall screen layout metrics shared by draw and focus.

use crate::core::tile::Suit;
use crate::render::theme::{metrics, typography};
use crate::ui::controller_hints::{HintStyle, screen_footer_top};
use crate::ui::widget::PLAIN_TEXT_LINE_STEP_MUL;

use crate::scenes::header_chrome::{HeaderChromeMetrics, HeaderTitleLayout};

pub const ROW_LABELS: [&str; 5] = ["MANZU", "SOUZU", "PINZU", "HONORS", "FLOWERS"];
pub const GRID_ROWS: [(usize, usize); 5] = [(0, 9), (9, 9), (18, 9), (27, 7), (34, 4)];
const GRID_COLS: usize = 9;
const ROW_HEIGHT_FRAC: [f32; 5] = [0.92, 0.92, 0.90, 0.78, 0.72];

/// Warm candle-gold focus accent shared by grid cell and detail preview.
pub const LEDGER_FOCUS_OUTLINE: [f32; 4] = [0.80, 0.65, 0.38, 0.78];
pub const LEDGER_FOCUS_GLOW: [f32; 4] = [0.85, 0.70, 0.42, 0.12];

#[inline]
pub fn text_line_h(font_px: f32) -> f32 {
    font_px * PLAIN_TEXT_LINE_STEP_MUL
}

/// Vertical space for the wall-summary remaining/total progress bar (counts + labels).
pub fn wall_progress_bar_block_h(layout: &WallLayout) -> f32 {
    let count_line = text_line_h(layout.count_px);
    let bar_h = (count_line * 0.38).max(10.0);
    let count_row = count_line.max(bar_h);
    let label_line = text_line_h(layout.caption_px);
    count_row + 2.0 + label_line
}

#[inline]
pub fn read_boost(window_w: f32, window_h: f32) -> f32 {
    (window_w.min(window_h) / 720.0).clamp(1.0, 1.38)
}

/// Reserve space through the footer hint row plus a gap so the bottom grid row
/// (and projected tile silhouettes) stay clear of [`push_screen_footer_hint`].
pub fn wall_footer_reserve(w: f32, h: f32) -> f32 {
    let style = HintStyle::standard(w, h);
    let gap = (h * 0.036).max(36.0);
    h - screen_footer_top(h, style) + gap
}

pub struct WallLayout {
    pub content_x: f32,
    pub content_w: f32,
    pub summary_x: f32,
    pub summary_w: f32,
    pub summary_y: f32,
    pub summary_h: f32,
    pub grid_x: f32,
    pub grid_y: f32,
    pub grid_w: f32,
    pub grid_h: f32,
    pub detail_y: f32,
    pub detail_h: f32,
    pub header_y: f32,
    pub header_h: f32,
    pub tab_y: f32,
    pub tab_h: f32,
    pub panel_top: f32,
    pub grid_pad: f32,
    pub grid_content_x: f32,
    pub grid_content_y: f32,
    pub grid_content_w: f32,
    pub grid_content_h: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    pub row_cell_h: [f32; 5],
    pub row_y: [f32; 5],
    pub cell_gap: f32,
    pub row_gap: f32,
    pub label_col_w: f32,
    pub label_gap: f32,
    pub title_px: f32,
    pub body_px: f32,
    pub small_px: f32,
    pub count_px: f32,
    pub caption_px: f32,
}

impl WallLayout {
    pub fn col_step(&self) -> f32 {
        self.cell_w + self.cell_gap
    }

    pub fn row_step(&self, row_idx: usize) -> f32 {
        self.row_cell_h[row_idx] + self.row_gap
    }

    pub fn summary_pad(&self) -> f32 {
        12.0
    }

    pub fn summary_value_x(&self) -> f32 {
        self.summary_x + self.summary_w - 14.0
    }

    pub fn row_width_for_cols(&self, count: usize) -> f32 {
        if count == 0 {
            return 0.0;
        }
        count as f32 * self.cell_w + (count.saturating_sub(1)) as f32 * self.cell_gap
    }

    pub fn row_center_offset(&self, count: usize) -> f32 {
        (self.row_width_for_cols(GRID_COLS) - self.row_width_for_cols(count)) * 0.5
    }

    pub fn tab_width(&self, tab_count: usize) -> f32 {
        (self.content_w * 0.52) / tab_count as f32
    }
}

/// Advance width heuristic for a single-line row label at `font_px`.
fn row_label_text_width(text: &str, font_px: f32) -> f32 {
    let chars = text.chars().count() as f32;
    (font_px * chars * 0.54).max(font_px * 2.2)
}

/// Column width for suit row headers — must fit `FLOWERS` without clipping.
pub fn row_label_col_width(w: f32, jr: f32, h: f32) -> f32 {
    let font_px = typography::tier_at_most(22.0 * jr, h);
    let text_need = row_label_text_width("FLOWERS", font_px);
    let dot_and_pad = 7.0 * jr + 8.0;
    let min_w = 76.0 * jr;
    let max_w = w * 0.058;
    let lo = min_w.min(max_w);
    let hi = min_w.max(max_w);
    (dot_and_pad + text_need + 6.0).clamp(lo, hi)
}

/// Font size for a row label constrained by column width and row height.
pub fn row_label_font_px(text_w: f32, cell_h: f32, h: f32) -> f32 {
    let cap_h = typography::tier_at_most(cell_h * 0.36, h);
    let floor = typography::size(typography::H45, h);
    let mut px = cap_h;
    while px > floor && row_label_text_width("FLOWERS", px) > text_w {
        px -= 0.5;
    }
    px.max(floor)
}

pub fn wall_layout(w: f32, h: f32, jr: f32) -> WallLayout {
    let back = HeaderChromeMetrics::from_window(w, h).back_rect_left();
    let scale = metrics::scene_scale(w, h);
    let title_px = typography::size(typography::H24, h);
    let body_px = typography::size(typography::H36, h);
    let small_px = typography::size(typography::H42, h);
    let count_px = typography::size(typography::H28, h);
    let caption_px = small_px;

    let content_x = w * 0.04;
    let content_w = w * 0.92;
    let title = HeaderTitleLayout::nav_row_aligned(
        back,
        content_x + w * 0.015,
        (12.0 * scale).max(8.0),
        title_px,
        jr,
    );

    let header_y = title.title_y - 2.0 * jr;
    let header_h = title.subtitle_y + text_line_h(small_px) - header_y + 1.0 * jr;
    let tab_y = header_y + header_h;
    let tab_h = 0.0;
    let panel_top = tab_y + (6.0 * jr).max(4.0);
    let footer_reserve = wall_footer_reserve(w, h);
    let panel_h = h - footer_reserve - panel_top - 6.0 * jr;

    let summary_w = content_w * 0.28;
    let summary_x = content_x;
    let grid_x = summary_x + summary_w + content_w * 0.008;
    let grid_w = content_x + content_w - grid_x;
    let grid_pad = (5.0 * jr).max(4.0);
    let label_col_w = row_label_col_width(w, jr, h);
    let label_gap = (3.0 * jr).max(2.0);
    let grid_content_x = grid_x + grid_pad;
    let grid_content_y = panel_top + grid_pad;
    let grid_content_w = grid_w - grid_pad * 2.0;
    let grid_content_h = panel_h - grid_pad * 2.0;
    let cell_gap = (1.25 * jr).max(1.0);
    let row_gap = (2.3 * jr).max(1.8);
    let slots_w = grid_content_w - label_col_w - label_gap;
    let cell_w = (slots_w - cell_gap * (GRID_COLS - 1) as f32) / GRID_COLS as f32;
    let gap_total = row_gap * (GRID_ROWS.len() - 1) as f32;
    let weight_sum: f32 = ROW_HEIGHT_FRAC.iter().sum();
    let unit_h = (grid_content_h - gap_total) / weight_sum;
    let row_cell_h = std::array::from_fn(|i| (unit_h * ROW_HEIGHT_FRAC[i]).max(46.0 * jr));
    let mut row_y = [0.0_f32; 5];
    row_y[0] = grid_content_y;
    for i in 1..5 {
        row_y[i] = row_y[i - 1] + row_cell_h[i - 1] + row_gap;
    }
    let cell_h = row_cell_h[0];

    let summary_h = panel_h;
    let detail_y = panel_top + panel_h;
    let detail_h = 0.0;

    WallLayout {
        content_x,
        content_w,
        summary_x,
        summary_w,
        summary_y: panel_top,
        summary_h,
        grid_x,
        grid_y: panel_top,
        grid_w,
        grid_h: panel_h,
        detail_y,
        detail_h,
        header_y,
        header_h,
        tab_y,
        tab_h,
        panel_top,
        grid_pad,
        grid_content_x,
        grid_content_y,
        grid_content_w,
        grid_content_h,
        cell_w,
        cell_h,
        row_cell_h,
        row_y,
        cell_gap,
        row_gap,
        label_col_w,
        label_gap,
        title_px,
        body_px,
        small_px,
        count_px,
        caption_px,
    }
}

pub fn grid_cell_rect(layout: &WallLayout, face_idx: usize) -> Option<[f32; 4]> {
    let mut cursor = 0usize;
    for (row_idx, &(_start, count)) in GRID_ROWS.iter().enumerate() {
        if face_idx < cursor + count {
            let col = face_idx - cursor;
            let row_y = layout.row_y[row_idx];
            let cell_h = layout.row_cell_h[row_idx];
            let row_offset = layout.row_center_offset(count);
            let x = layout.grid_content_x
                + layout.label_col_w
                + layout.label_gap
                + row_offset
                + col as f32 * layout.col_step();
            return Some([x, row_y, layout.cell_w, cell_h]);
        }
        cursor += count;
    }
    None
}

pub fn grid_row_rect(layout: &WallLayout, row_idx: usize) -> [f32; 4] {
    [
        layout.grid_content_x,
        layout.row_y[row_idx],
        layout.grid_content_w,
        layout.row_cell_h[row_idx],
    ]
}

pub fn row_suit_color(row_idx: usize) -> [f32; 4] {
    match row_idx {
        0 => Suit::Manzu.keyword_color(),
        1 => Suit::Souzu.keyword_color(),
        2 => Suit::Pinzu.keyword_color(),
        3 => Suit::Wind.keyword_color(),
        _ => Suit::Flower.keyword_color(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honors_row_is_centered_in_nine_column_slot() {
        let layout = wall_layout(1920.0, 1080.0, 1.0);
        let first_honor = grid_cell_rect(&layout, 27).expect("honor cell");
        let last_honor = grid_cell_rect(&layout, 33).expect("honor cell");
        let row = grid_row_rect(&layout, 3);
        let slots_left = layout.grid_content_x + layout.label_col_w + layout.label_gap;
        let slots_w = layout.row_width_for_cols(GRID_COLS);
        let slot_center = slots_left + slots_w * 0.5;
        let center = (first_honor[0] + last_honor[0] + last_honor[2]) * 0.5;
        assert!((center - slot_center).abs() < 1.0);
        assert!(first_honor[0] >= row[0] - 0.5);
    }

    #[test]
    fn summary_panel_width_in_target_range() {
        let layout = wall_layout(1920.0, 1080.0, 1.0);
        let frac = layout.summary_w / layout.content_w;
        assert!(frac >= 0.26 && frac <= 0.30);
    }

    #[test]
    fn row_label_column_fits_flowers() {
        let layout = wall_layout(1920.0, 1080.0, 1.0);
        let text_w = layout.label_col_w - 12.0;
        let font_px = row_label_font_px(text_w, layout.row_cell_h[4], 1080.0);
        assert!(row_label_text_width("FLOWERS", font_px) <= text_w + 1.0);
        assert!(row_label_text_width("HONORS", font_px) <= text_w + 1.0);
    }

    #[test]
    fn row_label_column_width_on_narrow_window() {
        // Regression: min (76*jr) can exceed max (w*0.058) on ~1366px-wide windows.
        let w = 1366.0;
        let h = 768.0;
        let jr = read_boost(w, h);
        let layout = wall_layout(w, h, jr);
        assert!(layout.label_col_w.is_finite());
        assert!(layout.label_col_w > 0.0);
    }

    #[test]
    fn row_heights_fill_grid_content() {
        for (w, h) in [(1920.0, 1080.0), (2560.0, 1440.0), (3840.0, 2160.0)] {
            let jr = read_boost(w, h);
            let layout = wall_layout(w, h, jr);
            let gap_total = layout.row_gap * (GRID_ROWS.len() - 1) as f32;
            let rows_total: f32 = layout.row_cell_h.iter().sum();
            let used = rows_total + gap_total;
            assert!(
                (used - layout.grid_content_h).abs() < 1.5,
                "{w}x{h}: rows used {used:.1} of grid_content_h {:.1} (row_cell_h={:?})",
                layout.grid_content_h,
                layout.row_cell_h
            );
        }
    }

    #[test]
    fn vertical_budget_targets_main_content() {
        let layout = wall_layout(1920.0, 1080.0, 1.0);
        let header_tabs = layout.panel_top;
        let main = layout.grid_h;
        let footer = wall_footer_reserve(1920.0, 1080.0);
        let used = header_tabs + main + footer;
        assert!(used <= 1080.0 + 1.0);
        assert!(main / 1080.0 > 0.48);
        assert!(footer / 1080.0 > 0.08);
        assert!(footer / 1080.0 < 0.17);
        assert!(layout.summary_h / 1080.0 > 0.50);
    }
}
