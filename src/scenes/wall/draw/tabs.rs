//! Tab row attached to content panel + view-mode toggle.

use crate::game::wall_stats::WallCountView;
use crate::render::theme::color;
use crate::render::wgpu_renderer::TextAlign;

use super::super::focus::LedgerNav;
use super::super::layout::{view_toggle_rect, WallLayout};
use super::text::push_text;

pub fn draw_wall_tabs(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    view: WallCountView,
    focus: Option<LedgerNav>,
) {
    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: [
            layout.content_x,
            layout.tab_y + layout.tab_h - 1.0,
            layout.content_w,
            1.0,
        ],
        color: color::alpha(color::STONE, 0.16),
        user: 0,
    });

    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: [
            layout.content_x,
            layout.panel_top,
            layout.content_w,
            1.0,
        ],
        color: color::alpha(color::STONE, 0.18),
        user: 0,
    });

    let view_rect = view_toggle_rect(0.0, layout);
    let view_focused = focus == Some(LedgerNav::View);
    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: view_rect,
        color: color::alpha(color::WALNUT_DEEP, if view_focused { 0.65 } else { 0.40 }),
        user: 0,
    });

    let label_w = 38.0;
    push_text(
        texts,
        [view_rect[0] + 6.0, view_rect[1], label_w, view_rect[3]],
        "View:",
        layout.caption_px,
        color::STONE,
        false,
        TextAlign::Left,
    );

    let modes = WallCountView::ALL;
    let pill_gap = 2.0;
    let pills_w = view_rect[2] - label_w - 10.0;
    let pill_w = (pills_w - pill_gap * (modes.len() as f32 - 1.0)) / modes.len() as f32;
    let pill_y = view_rect[1] + 3.0;
    let pill_h = view_rect[3] - 6.0;
    for (i, mode) in modes.iter().enumerate() {
        let px = view_rect[0] + label_w + 4.0 + i as f32 * (pill_w + pill_gap);
        let active = view == *mode;
        if active {
            frame.quad(crate::render::wgpu_renderer::GpuInstance {
                rect: [px, pill_y, pill_w, pill_h],
                color: color::alpha(color::BRASS, 0.24),
                user: 0,
            });
        }
        push_text(
            texts,
            [px, pill_y, pill_w, pill_h],
            mode.label(),
            layout.caption_px * 0.94,
            if active {
                color::CHAMPAGNE
            } else {
                color::alpha(color::UMBER, 0.72)
            },
            active,
            TextAlign::Center,
        );
    }
}
