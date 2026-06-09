//! Wall screen header: title, remaining badge, optional stamp.

use crate::game::wall_stats::WallStats;
use crate::render::theme::{color, metrics};
use crate::render::wgpu_renderer::TextAlign;
use crate::scenes::header_chrome::{HeaderChromeMetrics, HeaderTitleLayout};

use super::super::layout::{text_line_h, WallLayout};
use super::text::push_text;

pub fn draw_wall_header(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    w: f32,
    h: f32,
    jr: f32,
    layout: &WallLayout,
    stats: &WallStats,
) {
    let back = HeaderChromeMetrics::from_window(w, h).back_rect_left();
    let title = HeaderTitleLayout::nav_row_aligned(
        back,
        layout.content_x + w * 0.015,
        (12.0 * metrics::scene_scale(w, h)).max(8.0),
        layout.title_px,
        jr,
    );

    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: [
            layout.content_x,
            layout.header_y,
            layout.content_w,
            layout.header_h,
        ],
        color: color::alpha(color::WALNUT_DEEP, 0.32),
        user: 0,
    });
    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: [
            layout.content_x,
            layout.header_y + layout.header_h - 1.0,
            layout.content_w * 0.62,
            1.0,
        ],
        color: color::alpha(color::STONE, 0.18),
        user: 0,
    });

    push_text(
        texts,
        [title.copy_x, title.title_y, w * 0.34, text_line_h(layout.title_px)],
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

    let remain_w = layout.content_w * 0.28;
    let remain_x = layout.content_x + layout.content_w - remain_w;
    push_text(
        texts,
        [remain_x, title.title_y, remain_w, text_line_h(layout.body_px)],
        format!("{} / {} remaining", stats.total_remaining, stats.total_wall),
        layout.body_px * 0.96,
        color::CHAMPAGNE,
        true,
        TextAlign::Right,
    );
}
