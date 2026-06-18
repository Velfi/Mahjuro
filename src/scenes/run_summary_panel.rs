//! Shared run-summary card for victory and defeat game-over screens.

use std::time::Instant;

use crate::core::progression::{POINTS_PER_LEVEL, meta_depth_roman};
use crate::render::decal::load_ui_font;
use crate::render::draw_cmd::{ImageQuad, ImageQuadSource, UiFrame};
use crate::render::theme::{color, typography};
use crate::render::vocabulary_colors::GlossaryMode;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextBlockVerticalAlign, TextLabel};
use crate::scenes::DrawCtx;
use crate::ui::clip::intersect_rect;
use crate::ui::controller_hints::{
    HintSegment, HintStyle, confirm_continue_footer_row, push_inline_hint_rows,
};
use crate::ui::input::InputMode;
use crate::ui::smooth_scroll::SmoothScroll;
use crate::ui::styled_text::{StyledBlockStyle, styled_line_block_height_at_font_px};
use crate::ui::widget::{PLAIN_TEXT_LINE_STEP_MUL, wrap_text};

#[derive(Clone, Debug)]
pub struct RunSummaryStats {
    pub best_structure: String,
    pub most_played_structure: String,
    pub total_score: String,
    pub completion: String,
}

#[derive(Clone, Debug)]
pub struct RunSummaryPanelLevel {
    pub current_level: u32,
    pub prev_level: u32,
    pub points_earned: u32,
    pub into_level: u32,
    pub progress_label: String,
    pub progress_value: String,
    pub level_transition: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RunSummaryPanelContent {
    pub headline: String,
    pub subtitle: String,
    /// Narrative at the top of the scroll panel, before depth (defeat reason or victory status).
    pub pre_depth_text: Option<String>,
    pub hint: String,
    pub stats_rows: Vec<(String, String)>,
    pub level: RunSummaryPanelLevel,
}

#[derive(Clone, Copy, Debug)]
pub struct RunSummaryPanelTheme {
    pub panel_x_frac: f32,
    pub anchor_right: bool,
    pub headline_color: [f32; 4],
    pub rule_color: [f32; 4],
    /// Glossary-tinted styled markup on the headline (defeat thank-you line).
    pub headline_styled: bool,
}

/// Shared panel width for victory and defeat game-over screens.
const PANEL_WIDTH_FRAC: f32 = 0.28;
/// Fixed subtitle slot height (in line heights) so panel size stays consistent.
const SUBTITLE_SLOT_LINES: f32 = 2.5;
/// Footer hints are smaller than the global standard and span the full window.
const RUN_SUMMARY_HINT_SCALE: f32 = 0.68;

#[inline]
fn run_summary_headline_font_px(h: f32, theme: &RunSummaryPanelTheme) -> f32 {
    if theme.headline_styled {
        typography::size(typography::H12, h)
    } else {
        typography::size(typography::H5, h)
    }
}

fn run_summary_hint_style(window_w: f32, h: f32) -> HintStyle {
    let mut style = HintStyle::standard(window_w, h);
    style.font_px *= RUN_SUMMARY_HINT_SCALE;
    style.line_h *= RUN_SUMMARY_HINT_SCALE;
    style.icon_px *= RUN_SUMMARY_HINT_SCALE;
    style.gap_after_icon *= RUN_SUMMARY_HINT_SCALE;
    style
}

impl RunSummaryPanelTheme {
    pub fn victory() -> Self {
        Self {
            panel_x_frac: 0.04,
            anchor_right: true,
            headline_color: color::CHAMPAGNE,
            rule_color: color::alpha(color::CHAMPAGNE, 0.35),
            headline_styled: true,
        }
    }

    pub fn defeat() -> Self {
        Self {
            panel_x_frac: 0.08,
            anchor_right: false,
            headline_color: [0.72, 0.09, 0.16, 1.0],
            rule_color: color::alpha(color::RUBY, 0.35),
            headline_styled: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunSummaryPanelLayout {
    pub window_h: f32,
    pub panel_rect: [f32; 4],
    pub headline_rect: [f32; 4],
    pub subtitle_rect: [f32; 4],
    pub rule_rect: [f32; 4],
    pub hint_flavor_rect: Option<[f32; 4]>,
    pub hint_rect: [f32; 4],
    pub level_rect: [f32; 4],
    pub level_inner_rect: [f32; 4],
    pub depth_title_rect: [f32; 4],
    pub points_chip_rect: [f32; 4],
    pub deeper_ribbon_rect: Option<[f32; 4]>,
    pub transition_rect: Option<[f32; 4]>,
    pub progress_label_rect: [f32; 4],
    pub progress_value_rect: [f32; 4],
    pub well_draw_rect: [f32; 4],
    pub stats_row_rects: Vec<[f32; 4]>,
    pub row_font_px: f32,
    pub row_line_h: f32,
    pub level_body_font: f32,
    pub level_title_font: f32,
    pub points_chip_font: f32,
    pub chip_detail_font: f32,
    pub text_align: TextAlign,
    /// Clip rect for scrollable panel interior content.
    pub scroll_clip_rect: [f32; 4],
    pub scroll_content_h: f32,
    pub scroll_viewport_h: f32,
    pub max_scroll_px: f32,
    pub scrollbar_track: Option<[f32; 4]>,
    pub subtitle_slot_rect: [f32; 4],
    pub pre_depth_text_rect: Option<[f32; 4]>,
}

/// Smooth scroll state for an overflowing run-summary panel.
pub struct RunSummaryPanelScroll {
    scroll: SmoothScroll,
    dragging_scrollbar: bool,
    scroll_drag_grab_y: f32,
    prev_mouse_down: bool,
}

impl Default for RunSummaryPanelScroll {
    fn default() -> Self {
        Self::new()
    }
}

impl RunSummaryPanelScroll {
    pub fn new() -> Self {
        Self {
            scroll: SmoothScroll::new(),
            dragging_scrollbar: false,
            scroll_drag_grab_y: 0.0,
            prev_mouse_down: false,
        }
    }

    pub fn sync(&self, layout: &RunSummaryPanelLayout) {
        self.scroll.set_max(layout.max_scroll_px.ceil() as u32);
    }

    pub fn handle_wheel(
        &self,
        scroll_lines: f32,
        cursor: (f32, f32),
        layout: &RunSummaryPanelLayout,
        input_mode: InputMode,
    ) {
        if scroll_lines.abs() <= 0.001 || layout.max_scroll_px <= 0.0 {
            return;
        }
        if input_mode == InputMode::Cursor && !cursor_over_scroll_area(cursor, layout) {
            return;
        }
        let line_h = layout.row_line_h;
        self.scroll.scroll_by(scroll_lines * line_h);
    }

    pub fn offset_px(&self) -> f32 {
        self.scroll.tick()
    }

    /// Handle scrollbar thumb/track clicks and drags. Returns `true` when the
    /// pointer gesture should not dismiss the screen (full-screen click target).
    pub fn handle_mouse(
        &mut self,
        cursor: (f32, f32),
        mouse_left_down: bool,
        layout: &RunSummaryPanelLayout,
    ) -> bool {
        let mouse_click = mouse_left_down && !self.prev_mouse_down;
        let mut block_dismiss = false;

        let Some(track) = layout.scrollbar_track else {
            if !mouse_left_down {
                self.dragging_scrollbar = false;
            }
            self.prev_mouse_down = mouse_left_down;
            return false;
        };

        let scroll_offset = self.scroll.target();
        let thumb = scrollbar_thumb(
            track,
            layout.scroll_content_h,
            layout.scroll_viewport_h,
            scroll_offset,
            layout.max_scroll_px,
            layout.row_font_px,
        );
        let hit_track = scrollbar_hit_track(track, layout.row_font_px);
        let (mx, my) = cursor;

        if self.dragging_scrollbar && mouse_left_down {
            let px = scroll_px_from_cursor(
                my,
                self.scroll_drag_grab_y,
                track,
                thumb[3],
                layout.max_scroll_px,
            );
            self.scroll.set_target(px);
            block_dismiss = true;
        } else if mouse_click {
            if point_in_rect(mx, my, thumb) {
                self.dragging_scrollbar = true;
                self.scroll_drag_grab_y = my - thumb[1];
                let px = scroll_px_from_cursor(
                    my,
                    self.scroll_drag_grab_y,
                    track,
                    thumb[3],
                    layout.max_scroll_px,
                );
                self.scroll.set_target(px);
                block_dismiss = true;
            } else if point_in_rect(mx, my, hit_track) {
                self.dragging_scrollbar = true;
                self.scroll_drag_grab_y = thumb[3] * 0.5;
                let px = scroll_px_from_cursor(
                    my,
                    self.scroll_drag_grab_y,
                    track,
                    thumb[3],
                    layout.max_scroll_px,
                );
                self.scroll.set_target(px);
                block_dismiss = true;
            }
        }

        if !mouse_left_down {
            self.dragging_scrollbar = false;
        }
        self.prev_mouse_down = mouse_left_down;
        block_dismiss
    }
}

#[inline]
fn point_in_rect(mx: f32, my: f32, r: [f32; 4]) -> bool {
    mx >= r[0] && mx <= r[0] + r[2] && my >= r[1] && my <= r[1] + r[3]
}

fn scrollbar_hit_track(track: [f32; 4], row_font_px: f32) -> [f32; 4] {
    let hit_pad_x = (row_font_px * 0.36).max(6.0);
    [
        track[0] - hit_pad_x,
        track[1],
        track[2] + hit_pad_x * 2.0,
        track[3],
    ]
}

fn scroll_px_from_cursor(
    my: f32,
    grab_y: f32,
    track: [f32; 4],
    thumb_h: f32,
    max_scroll_px: f32,
) -> f32 {
    let thumb_travel = (track[3] - thumb_h).max(0.0);
    if thumb_travel <= 0.0 {
        return 0.0;
    }
    let thumb_top = (my - grab_y - track[1]).clamp(0.0, thumb_travel);
    (thumb_top / thumb_travel) * max_scroll_px
}

fn shift_y(rect: [f32; 4], dy: f32) -> [f32; 4] {
    [rect[0], rect[1] - dy, rect[2], rect[3]]
}

fn push_clipped_quad(frame: &mut UiFrame, rect: [f32; 4], color: [f32; 4], clip: [f32; 4]) {
    if let Some(clipped) = intersect_rect(rect, clip) {
        frame.quad(GpuInstance {
            rect: clipped,
            color,
            user: 0,
        });
    }
}

fn cursor_over_scroll_area(cursor: (f32, f32), layout: &RunSummaryPanelLayout) -> bool {
    let (cx, cy) = cursor;
    let clip = layout.scroll_clip_rect;
    if cx >= clip[0] && cx <= clip[0] + clip[2] && cy >= clip[1] && cy <= clip[1] + clip[3] {
        return true;
    }
    layout.scrollbar_track.is_some_and(|track| {
        cx >= track[0] && cx <= track[0] + track[2] && cy >= track[1] && cy <= track[1] + track[3]
    })
}

fn scrollbar_thumb(
    track: [f32; 4],
    scroll_content_h: f32,
    scroll_viewport_h: f32,
    scroll_offset_px: f32,
    max_scroll_px: f32,
    row_font_px: f32,
) -> [f32; 4] {
    let track_h = track[3];
    let thumb_h = (track_h * (scroll_viewport_h / scroll_content_h.max(1.0)))
        .clamp(row_font_px * 0.75, track_h);
    let thumb_travel = (track_h - thumb_h).max(0.0);
    let thumb_t = if max_scroll_px > 0.0 {
        scroll_offset_px / max_scroll_px
    } else {
        0.0
    };
    [
        track[0],
        track[1] + thumb_travel * thumb_t,
        track[2],
        thumb_h,
    ]
}

fn push_scrollbar(frame: &mut UiFrame, layout: &RunSummaryPanelLayout, scroll_offset_px: f32) {
    let Some(track) = layout.scrollbar_track else {
        return;
    };
    frame.quad(GpuInstance {
        rect: track,
        color: color::alpha(color::WALNUT_INK, 0.45),
        user: 0,
    });
    let thumb = scrollbar_thumb(
        track,
        layout.scroll_content_h,
        layout.scroll_viewport_h,
        scroll_offset_px,
        layout.max_scroll_px,
        layout.row_font_px,
    );
    frame.quad(GpuInstance {
        rect: thumb,
        color: color::alpha(color::CHAMPAGNE, 0.82),
        user: 0,
    });
}

/// Depth-well fill states — one composite sprite per step (0 = empty … 5 = full).
const DEPTH_WELL_LAYER_ASSETS: [&str; 6] = [
    "textures/depth_well/depth_well_0.png",
    "textures/depth_well/depth_well_1.png",
    "textures/depth_well/depth_well_2.png",
    "textures/depth_well/depth_well_3.png",
    "textures/depth_well/depth_well_4.png",
    "textures/depth_well/depth_well_5.png",
];

/// Normalized UV bounds of non-transparent pixels shared by all depth-well layers.
const DEPTH_WELL_OPAQUE_UV: [f32; 4] = [0.017, 0.155, 0.982, 0.819];

const DEPTH_WELL_MAX_HEIGHT_MUL: f32 = 6.0;

fn text_width(text: &str, font_px: f32) -> f32 {
    let Some(font) = load_ui_font() else {
        return text.chars().count() as f32 * font_px * 0.52;
    };
    text.chars()
        .map(|ch| font.metrics(ch, font_px).advance_width)
        .sum()
}

fn stats_column_widths(rows: &[(String, String)], inner_w: f32, font_px: f32) -> (f32, f32, f32) {
    let gap = (font_px * 0.30).max(8.0);
    let widest_label = rows
        .iter()
        .map(|(label, _)| text_width(label, font_px))
        .fold(0.0_f32, f32::max);
    let label_w = (widest_label + font_px * 0.25).min(inner_w * 0.45);
    let value_w = (inner_w - label_w - gap).max(inner_w * 0.35);
    let value_x = label_w + gap;
    (label_w, value_x, value_w)
}

fn wrapped_row_height(text: &str, col_w: f32, font_px: f32, line_h: f32) -> f32 {
    let lines = wrap_text(text, col_w, font_px / 0.99);
    line_h * lines.len().max(1) as f32
}

fn depth_well_size(inner_w: f32, inset: f32, row_line_h: f32) -> (f32, f32) {
    let uw = DEPTH_WELL_OPAQUE_UV[2] - DEPTH_WELL_OPAQUE_UV[0];
    let vh = DEPTH_WELL_OPAQUE_UV[3] - DEPTH_WELL_OPAQUE_UV[1];
    let max_w = (inner_w - inset * 2.0).max(0.0);
    let max_h = row_line_h * DEPTH_WELL_MAX_HEIGHT_MUL;
    let mut w = max_w;
    let mut h = w * vh / uw;
    if h > max_h {
        h = max_h;
        w = h * uw / vh;
    }
    (w, h)
}

fn depth_well_draw_rect(viewport: [f32; 4]) -> [f32; 4] {
    let [u0, v0, u1, v1] = DEPTH_WELL_OPAQUE_UV;
    let uw = u1 - u0;
    let vh = v1 - v0;
    let side = viewport[2] / uw;
    let opaque_h = side * vh;
    [
        viewport[0] - side * u0,
        viewport[1] + (viewport[3] - opaque_h) * 0.5 - side * v0,
        side,
        side,
    ]
}

impl RunSummaryPanelLayout {
    pub fn compute(
        w: f32,
        h: f32,
        content: &RunSummaryPanelContent,
        theme: &RunSummaryPanelTheme,
    ) -> Self {
        let row_font_px = typography::size(typography::H36, h);
        let row_line_h = row_font_px * PLAIN_TEXT_LINE_STEP_MUL;
        let level_body_font = typography::size(typography::H42, h);
        let level_title_font = typography::size(typography::H24, h);
        let points_chip_font = typography::size(typography::H42, h);
        let chip_detail_font = typography::size(typography::H45, h);
        let pad_v = row_font_px * 0.8;
        let pad_h = row_font_px * 0.55;
        let scroll_bottom_pad = row_font_px * 0.45;
        let panel_w = w * PANEL_WIDTH_FRAC;
        let content_w = panel_w;
        let level_well_inset = row_font_px * 0.32;
        let level_inner_w_est = panel_w - 12.0;
        let (_, well_height_px) = depth_well_size(level_inner_w_est, level_well_inset, row_line_h);

        let has_transition = content.level.level_transition.is_some();
        let leveled_up = content.level.current_level > content.level.prev_level;
        let header_h = row_line_h * 1.30;
        let transition_h = if has_transition {
            row_line_h * 1.05
        } else {
            0.0
        };
        let progress_h = row_line_h * 1.05;
        let well_gap = row_font_px * 0.28;
        let level_header_h = header_h + transition_h + progress_h + well_gap;
        let level_block_h = level_header_h + well_height_px + row_font_px * 0.22;
        let level_block_gap = row_font_px * 0.60;
        let scrollbar_track_w = (row_font_px * 0.22).max(4.0);
        let scrollbar_gutter_est = scrollbar_track_w + row_font_px * 0.36;
        let scroll_inner_w = (panel_w - pad_h * 2.0 - scrollbar_gutter_est).max(0.0);
        let (label_col_w, _, value_col_w) =
            stats_column_widths(&content.stats_rows, scroll_inner_w, row_font_px);
        let stats_h = content
            .stats_rows
            .iter()
            .map(|(label, value)| {
                wrapped_row_height(label, label_col_w, row_font_px, row_line_h).max(
                    wrapped_row_height(value, value_col_w, row_font_px, row_line_h),
                )
            })
            .sum::<f32>();
        let panel_x = if theme.anchor_right {
            w - panel_w - w * theme.panel_x_frac
        } else {
            w * theme.panel_x_frac
        };

        let gap = row_font_px * 0.5;
        let sub_font = typography::size(typography::H32, h);
        let sub_line_h = sub_font * 1.3;
        let headline_font = run_summary_headline_font_px(h, theme);
        let headline_measure_w = if theme.headline_styled { w } else { content_w };
        let headline_h = if theme.headline_styled {
            styled_line_block_height_at_font_px(
                &content.headline,
                headline_measure_w,
                headline_font,
                GlossaryMode::Prose,
                color::CHAMPAGNE,
            )
        } else {
            headline_font * 1.25
        };
        let top_pad = if theme.headline_styled {
            (h * 0.012).max(row_font_px * 0.2)
        } else {
            (h * 0.04).max(row_font_px * 0.6)
        };
        let min_top_pad = top_pad;
        let headline_panel_gap = if theme.headline_styled {
            row_font_px * 0.22
        } else {
            gap
        };
        const RULE_H: f32 = 2.0;
        let headline_rule_h = if theme.headline_styled { 0.0 } else { RULE_H };
        let has_subtitle = !content.subtitle.is_empty();
        let subtitle_slot_h = if has_subtitle {
            sub_line_h * SUBTITLE_SLOT_LINES
        } else {
            0.0
        };
        let pre_depth_gap = if content.pre_depth_text.is_some() {
            row_font_px * 0.45
        } else {
            0.0
        };
        let pre_depth_text_h = content
            .pre_depth_text
            .as_ref()
            .map(|text| {
                wrapped_row_height(text, scroll_inner_w, sub_font, sub_line_h) + pre_depth_gap
            })
            .unwrap_or(0.0);

        let hint_style = run_summary_hint_style(w, h);
        let hint_h = hint_style.line_h;
        let hint_row_gap = hint_h * 0.20;
        let hint_rows = if content.hint.is_empty() { 1.0 } else { 2.0 };
        let hint_stack_h = hint_h * hint_rows + hint_row_gap * (hint_rows - 1.0);
        let bottom_pad = (h * 0.035).max(row_font_px * 0.55);

        let scroll_content_h =
            pre_depth_text_h + level_block_h + level_block_gap + stats_h + scroll_bottom_pad;
        let content_panel_h = scroll_content_h + pad_v * 2.0;

        let (panel_y, panel_h, headline_y, sub_y, rule_y) = if theme.headline_styled {
            let headline_band_h = headline_h
                + if has_subtitle {
                    gap + subtitle_slot_h + gap
                } else {
                    0.0
                }
                + headline_rule_h
                + headline_panel_gap;
            let headline_reserve = headline_band_h + min_top_pad * 2.0;
            let panel_h = content_panel_h.min((h - headline_reserve).max(row_line_h * 3.0));
            let panel_y = (h - panel_h) * 0.5;
            let headline_y = if has_subtitle {
                let block_h = headline_h + gap + subtitle_slot_h;
                ((panel_y - block_h) * 0.5).max(min_top_pad)
            } else {
                ((panel_y - headline_h) * 0.5).max(min_top_pad)
            };
            let sub_y = headline_y + headline_h + if has_subtitle { gap } else { 0.0 };
            let rule_y = if has_subtitle {
                sub_y + subtitle_slot_h + gap
            } else {
                headline_y + headline_h + headline_panel_gap
            };
            (panel_y, panel_h, headline_y, sub_y, rule_y)
        } else {
            let fixed_top = top_pad
                + headline_h
                + if has_subtitle {
                    gap + subtitle_slot_h + gap
                } else {
                    0.0
                }
                + headline_rule_h
                + headline_panel_gap;
            let available_panel_h = (h - fixed_top - bottom_pad).max(row_line_h * 3.0);
            let panel_h = content_panel_h.min(available_panel_h);
            let panel_y = fixed_top;
            let headline_y = top_pad;
            let sub_y = top_pad + headline_h + if has_subtitle { gap } else { 0.0 };
            let rule_y = if has_subtitle {
                sub_y + subtitle_slot_h + gap
            } else {
                top_pad + headline_h + headline_panel_gap
            };
            (panel_y, panel_h, headline_y, sub_y, rule_y)
        };

        let scroll_viewport_h = (panel_h - pad_v * 2.0).max(0.0);
        let max_scroll_px = (scroll_content_h - scroll_viewport_h).max(0.0);

        let panel_rect = [panel_x, panel_y, panel_w, panel_h];
        let scrollbar_gutter = if max_scroll_px > 0.0 {
            scrollbar_gutter_est
        } else {
            0.0
        };
        let scroll_inner_w = (panel_w - pad_h * 2.0 - scrollbar_gutter).max(0.0);
        let (label_col_w, _, value_col_w) =
            stats_column_widths(&content.stats_rows, scroll_inner_w, row_font_px);
        let scroll_content_x = panel_x + pad_h;
        let scroll_clip_rect = [
            scroll_content_x,
            panel_rect[1] + pad_v,
            scroll_inner_w,
            scroll_viewport_h,
        ];
        let scrollbar_track = if max_scroll_px > 0.0 {
            let track_pad = row_font_px * 0.18;
            Some([
                panel_rect[0] + panel_rect[2] - scrollbar_track_w - track_pad,
                panel_rect[1] + pad_v,
                scrollbar_track_w,
                scroll_viewport_h,
            ])
        } else {
            None
        };
        let subtitle_slot_rect = [panel_x, sub_y, content_w, subtitle_slot_h];
        let headline_x = if theme.anchor_right {
            panel_x + panel_w - content_w
        } else {
            panel_x
        };
        let headline_rect = if theme.headline_styled {
            [0.0, headline_y, w, headline_h]
        } else {
            [headline_x, headline_y, content_w, headline_h]
        };
        let rule_rect = if theme.headline_styled {
            [panel_x, rule_y, panel_w, 0.0]
        } else {
            [panel_x, rule_y, panel_w, RULE_H]
        };

        let hint_base_y = (h - hint_stack_h - hint_h * 0.22).max(0.0);
        let hint_flavor_rect = (!content.hint.is_empty()).then_some([0.0, hint_base_y, w, hint_h]);
        let hint_rect = [
            0.0,
            hint_base_y
                + if hint_flavor_rect.is_some() {
                    hint_h + hint_row_gap
                } else {
                    0.0
                },
            w,
            hint_h,
        ];

        let inner_x = scroll_content_x;
        let inner_w = scroll_inner_w;
        let inner_y = panel_rect[1] + pad_v;
        let pre_depth_text_rect = content.pre_depth_text.as_ref().map(|text| {
            let text_h = wrapped_row_height(text, scroll_inner_w, sub_font, sub_line_h);
            [scroll_content_x, inner_y, scroll_inner_w, text_h]
        });
        let level_y = inner_y + pre_depth_text_h;
        let level_rect = [scroll_content_x, level_y, scroll_inner_w, level_block_h];
        let level_inner_rect = [
            level_rect[0] + 3.0,
            level_rect[1] + 3.0,
            level_rect[2] - 6.0,
            level_rect[3] - 6.0,
        ];

        let side_pad = row_font_px * 0.32;
        let points_chip_w = level_inner_rect[2] * 0.34;
        let points_chip_h = row_line_h * 0.92;
        let deeper_ribbon_w = level_rect[2] * 0.30;
        let deeper_ribbon_h = row_line_h * 0.86;

        let row_y0 = level_inner_rect[1] + row_font_px * 0.12;
        let depth_title_rect = [
            level_inner_rect[0] + side_pad,
            row_y0,
            level_inner_rect[2] * 0.58,
            header_h,
        ];
        let deeper_ribbon_rect = if leveled_up {
            Some([
                level_inner_rect[0] + level_inner_rect[2] - deeper_ribbon_w - side_pad,
                row_y0 + (header_h - deeper_ribbon_h) * 0.5,
                deeper_ribbon_w,
                deeper_ribbon_h,
            ])
        } else {
            None
        };
        let chip_y = if leveled_up {
            row_y0 + header_h
        } else {
            row_y0 + (header_h - points_chip_h) * 0.5
        };
        let points_chip_rect = [
            level_inner_rect[0] + level_inner_rect[2] - points_chip_w - side_pad,
            chip_y,
            points_chip_w,
            points_chip_h,
        ];

        let row_y1 = row_y0 + header_h;
        let transition_rect = content.level.level_transition.as_ref().map(|_| {
            let chip_gap = row_font_px * 0.16;
            let transition_w = level_inner_rect[2] - side_pad * 2.0 - points_chip_w - chip_gap;
            [
                level_inner_rect[0] + side_pad,
                row_y1,
                transition_w.max(level_inner_rect[2] * 0.42),
                transition_h.max(points_chip_h),
            ]
        });

        let row_y2 = row_y1 + transition_h;
        let progress_label_rect = [
            level_inner_rect[0] + side_pad,
            row_y2,
            level_inner_rect[2] * 0.58,
            progress_h,
        ];
        let progress_value_rect = [
            level_inner_rect[0] + level_inner_rect[2] - level_inner_rect[2] * 0.32 - side_pad,
            row_y2,
            level_inner_rect[2] * 0.28,
            progress_h,
        ];

        let (well_width_px, well_height_px) =
            depth_well_size(level_inner_rect[2], level_well_inset, row_line_h);
        let well_slot_w = (level_inner_rect[2] - level_well_inset * 2.0).max(0.0);
        let well_x =
            level_inner_rect[0] + level_well_inset + (well_slot_w - well_width_px).max(0.0) * 0.5;
        let well_y = row_y2 + progress_h + well_gap;
        let well_viewport = [well_x, well_y, well_width_px, well_height_px];
        let well_draw_rect = depth_well_draw_rect(well_viewport);

        let mut stats_row_rects = Vec::with_capacity(content.stats_rows.len());
        let mut row_y = level_y + level_block_h + level_block_gap;
        for (label, value) in &content.stats_rows {
            let label_lines = wrap_text(label, label_col_w, row_font_px / 0.99);
            let value_lines = wrap_text(value, value_col_w, row_font_px / 0.99);
            let row_h = row_line_h * label_lines.len().max(value_lines.len()).max(1) as f32;
            stats_row_rects.push([inner_x, row_y, inner_w, row_h]);
            let _ = (label_lines, value_lines);
            row_y += row_h;
        }

        let text_align = if theme.anchor_right {
            TextAlign::Right
        } else {
            TextAlign::Left
        };

        Self {
            window_h: h,
            panel_rect,
            headline_rect,
            subtitle_rect: subtitle_slot_rect,
            rule_rect,
            hint_flavor_rect,
            hint_rect,
            level_rect,
            level_inner_rect,
            depth_title_rect,
            points_chip_rect,
            deeper_ribbon_rect,
            transition_rect,
            progress_label_rect,
            progress_value_rect,
            well_draw_rect,
            stats_row_rects,
            row_font_px,
            row_line_h,
            level_body_font,
            level_title_font,
            points_chip_font,
            chip_detail_font,
            text_align,
            scroll_clip_rect,
            scroll_content_h,
            scroll_viewport_h,
            max_scroll_px,
            scrollbar_track,
            subtitle_slot_rect,
            pre_depth_text_rect,
        }
    }
}

pub fn push_run_summary_panel(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    layout: &RunSummaryPanelLayout,
    content: &RunSummaryPanelContent,
    theme: &RunSummaryPanelTheme,
    opened_at: Instant,
    scroll_offset_px: f32,
) {
    let h = layout.window_h;
    let row_font_px = layout.row_font_px;
    let level_body_font = layout.level_body_font;
    let level_title_font = layout.level_title_font;
    let points_chip_font = layout.points_chip_font;
    let chip_detail_font = layout.chip_detail_font;
    let panel_rect = layout.panel_rect;
    let clip = layout.scroll_clip_rect;
    let inner_w = clip[2];
    let inner_x = clip[0];
    let (label_col_w, value_col_x, value_col_w) =
        stats_column_widths(&content.stats_rows, inner_w, row_font_px);
    let scroll = scroll_offset_px;

    let border_rect = [
        panel_rect[0] + 2.0,
        panel_rect[1] + 2.0,
        panel_rect[2] - 4.0,
        panel_rect[3] - 4.0,
    ];

    frame.quad(GpuInstance {
        rect: panel_rect,
        color: color::alpha(color::WALNUT_DEEP, 0.88),
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: border_rect,
        color: color::alpha(color::WALNUT_BRIGHT, 0.60),
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [
            panel_rect[0] + 3.0,
            panel_rect[1] + 3.0,
            panel_rect[2] - 6.0,
            panel_rect[3] - 6.0,
        ],
        color: color::alpha(color::WALNUT_DEEP, 0.88),
        user: 0,
    });

    let sub_font = typography::size(typography::H32, h);
    let headline_font = run_summary_headline_font_px(h, theme);

    if theme.headline_styled {
        let block = crate::ui::styled_text::StyledTextBlock::measure_at_font_px(
            &content.headline,
            layout.headline_rect[2],
            headline_font,
            GlossaryMode::Prose,
            color::CHAMPAGNE,
        );
        let mut headline_labels = Vec::new();
        block.push_at_font_px(
            &mut headline_labels,
            layout.headline_rect,
            StyledBlockStyle {
                color: color::CHAMPAGNE,
                align: TextAlign::Center,
                glossary: GlossaryMode::Prose,
                vertical_align: Some(TextBlockVerticalAlign::Top),
                ..Default::default()
            },
        );
        for label in headline_labels {
            frame.text(label);
        }
    } else {
        frame.text(TextLabel {
            rect: layout.headline_rect,
            text: content.headline.clone(),
            color: theme.headline_color,
            font_px: Some(headline_font),
            align: layout.text_align,
            ..Default::default()
        });
    }
    if !content.subtitle.is_empty() {
        let sub_lines = wrap_text(&content.subtitle, layout.subtitle_rect[2], sub_font / 0.99);
        frame.text(TextLabel {
            rect: layout.subtitle_rect,
            text: sub_lines.join("\n"),
            color: color::CHAMPAGNE,
            font_px: Some(sub_font),
            align: layout.text_align,
            clip_rect: Some(layout.subtitle_slot_rect),
            ..Default::default()
        });
    }
    if layout.rule_rect[3] > 0.0 {
        frame.quad(GpuInstance {
            rect: layout.rule_rect,
            color: theme.rule_color,
            user: 0,
        });
    }

    if let (Some(text), Some(rect)) = (&content.pre_depth_text, layout.pre_depth_text_rect) {
        let rect = shift_y(rect, scroll);
        let lines = wrap_text(text, rect[2], sub_font / 0.99);
        frame.text(TextLabel {
            rect,
            text: lines.join("\n"),
            color: color::CHAMPAGNE,
            font_px: Some(sub_font),
            align: layout.text_align,
            clip_rect: Some(clip),
            ..Default::default()
        });
    }

    let level_rect = shift_y(layout.level_rect, scroll);
    let level_border_rect = [
        level_rect[0] + 2.0,
        level_rect[1] + 2.0,
        level_rect[2] - 4.0,
        level_rect[3] - 4.0,
    ];
    push_clipped_quad(
        frame,
        level_rect,
        color::alpha(color::WALNUT_SOFT, 0.70),
        clip,
    );
    push_clipped_quad(
        frame,
        level_border_rect,
        color::alpha(color::WALNUT_BRIGHT, 0.75),
        clip,
    );
    push_clipped_quad(
        frame,
        shift_y(layout.level_inner_rect, scroll),
        color::alpha(color::WALNUT_RAISED, 0.72),
        clip,
    );

    frame.text(TextLabel {
        rect: shift_y(layout.depth_title_rect, scroll),
        text: format!("DEPTH {}", meta_depth_roman(content.level.current_level)),
        color: color::CHAMPAGNE,
        font_px: Some(level_title_font),
        align: TextAlign::Left,
        clip_rect: Some(clip),
        ..Default::default()
    });

    if let Some(ribbon_rect) = layout.deeper_ribbon_rect {
        let ribbon_rect = shift_y(ribbon_rect, scroll);
        push_clipped_quad(
            frame,
            ribbon_rect,
            color::alpha(color::WALNUT_SOFT, 0.96),
            clip,
        );
        push_clipped_quad(
            frame,
            [
                ribbon_rect[0] + 1.0,
                ribbon_rect[1] + 1.0,
                ribbon_rect[2] - 2.0,
                ribbon_rect[3] - 2.0,
            ],
            color::alpha(color::BRASS, 0.88),
            clip,
        );
        frame.text(TextLabel {
            rect: ribbon_rect,
            text: "DEEPER".to_string(),
            color: color::WALNUT_DEEP,
            font_px: Some(chip_detail_font),
            align: TextAlign::Center,
            clip_rect: Some(clip),
            ..Default::default()
        });
    }

    let points_chip_rect = shift_y(layout.points_chip_rect, scroll);
    push_clipped_quad(
        frame,
        points_chip_rect,
        color::alpha(color::WALNUT_BRIGHT, 0.85),
        clip,
    );
    frame.text(TextLabel {
        rect: points_chip_rect,
        text: format!("{} steps down", content.level.points_earned),
        color: color::PARCHMENT,
        font_px: Some(points_chip_font),
        align: TextAlign::Center,
        clip_rect: Some(clip),
        ..Default::default()
    });

    if let (Some(transition), Some(transition_rect)) =
        (&content.level.level_transition, layout.transition_rect)
    {
        frame.text(TextLabel {
            rect: shift_y(transition_rect, scroll),
            text: transition.clone(),
            color: color::alpha(color::GOLD, 0.92),
            font_px: Some(level_body_font),
            align: TextAlign::Left,
            clip_rect: Some(clip),
            ..Default::default()
        });
    }

    frame.text(TextLabel {
        rect: shift_y(layout.progress_label_rect, scroll),
        text: content.level.progress_label.clone(),
        color: color::STONE,
        font_px: Some(level_body_font),
        align: TextAlign::Left,
        clip_rect: Some(clip),
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: shift_y(layout.progress_value_rect, scroll),
        text: content.level.progress_value.clone(),
        color: color::PARCHMENT,
        font_px: Some(level_body_font),
        align: TextAlign::Right,
        clip_rect: Some(clip),
        ..Default::default()
    });

    for (idx, ((label, value), row_rect)) in content
        .stats_rows
        .iter()
        .zip(layout.stats_row_rects.iter())
        .enumerate()
    {
        let row_rect = shift_y(*row_rect, scroll);
        let label_lines = wrap_text(label, label_col_w, row_font_px / 0.99);
        let value_lines = wrap_text(value, value_col_w, row_font_px / 0.99);
        let row_h = row_rect[3];

        if idx % 2 == 0 {
            push_clipped_quad(
                frame,
                [clip[0], row_rect[1], clip[2], row_h],
                color::alpha(color::WALNUT_RAISED, 0.25),
                clip,
            );
        }

        frame.text(TextLabel {
            rect: [inner_x, row_rect[1], label_col_w, row_h],
            text: label_lines.join("\n"),
            color: color::STONE,
            font_px: Some(row_font_px),
            align: TextAlign::Left,
            clip_rect: Some(clip),
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [inner_x + value_col_x, row_rect[1], value_col_w, row_h],
            text: value_lines.join("\n"),
            color: color::PARCHMENT,
            font_px: Some(row_font_px),
            align: TextAlign::Right,
            clip_rect: Some(clip),
            ..Default::default()
        });
    }

    let mut hint_rects = Vec::with_capacity(2);
    let mut hint_rows = Vec::with_capacity(2);
    if let Some(flavor_rect) = layout.hint_flavor_rect {
        hint_rects.push(flavor_rect);
        hint_rows.push(vec![HintSegment::text(content.hint.clone())]);
    }
    hint_rects.push(layout.hint_rect);
    hint_rows.push(confirm_continue_footer_row(ctx.input_mode, ""));
    push_inline_hint_rows(
        frame,
        ctx,
        &hint_rects,
        &hint_rows,
        run_summary_hint_style(ctx.layout.window_w, h),
    );

    let elapsed = opened_at.elapsed().as_secs_f32();
    let displayed_fill = (elapsed * 4.0).min(content.level.into_level as f32);
    let sprite_idx = (displayed_fill.floor() as usize).min(POINTS_PER_LEVEL as usize);
    let well_rect = shift_y(layout.well_draw_rect, scroll);
    if intersect_rect(well_rect, clip).is_some() {
        frame.image_quads([ImageQuad {
            inst: GpuInstance {
                rect: well_rect,
                color: [1.0, 1.0, 1.0, 1.0],
                user: 0,
            },
            source: ImageQuadSource::Asset {
                path: DEPTH_WELL_LAYER_ASSETS[sprite_idx],
            },
            clip_rect: Some(clip),
        }]);
    }

    push_scrollbar(frame, layout, scroll_offset_px);
}
