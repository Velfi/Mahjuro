//! Wall screen header: title and subtitle.

use crate::render::theme::{color, metrics};
use crate::render::wgpu_renderer::TextAlign;
use crate::scenes::header_chrome::{HeaderChromeMetrics, HeaderTitleLayout};

use super::super::layout::{WallLayout, text_line_h};
use super::text::push_text;

pub fn draw_wall_header(
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    w: f32,
    h: f32,
    jr: f32,
    layout: &WallLayout,
) {
    let back = HeaderChromeMetrics::from_window(w, h).back_rect_left();
    let title = HeaderTitleLayout::nav_row_aligned(
        back,
        layout.content_x + w * 0.015,
        (12.0 * metrics::scene_scale(w, h)).max(8.0),
        layout.title_px,
        jr,
    );

    push_text(
        texts,
        [
            title.copy_x,
            title.title_y,
            w * 0.34,
            text_line_h(layout.title_px),
        ],
        "The Wall",
        layout.title_px,
        color::CHAMPAGNE,
        true,
        TextAlign::Left,
    );
    push_text(
        texts,
        [
            title.copy_x,
            title.subtitle_y,
            w * 0.34,
            text_line_h(layout.caption_px),
        ],
        "next round supply",
        layout.caption_px,
        color::STONE,
        false,
        TextAlign::Left,
    );
}
