//! Thin bottom help strip — text only, no oversized key glyphs.

use crate::ui::input::InputMode;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::TextAlign;

use super::super::layout::{text_line_h, wall_footer_reserve, WallLayout};
use super::text::push_text;

pub fn draw_wall_footer_controls(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    w: f32,
    h: f32,
    input_mode: InputMode,
) {
    let reserve = wall_footer_reserve(w, h);
    let line_h = reserve * 0.72;
    let y = h - reserve + (reserve - line_h) * 0.5;
    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: [0.0, h - reserve, w, reserve],
        color: color::alpha(color::WALNUT_INK, 0.35),
        user: 0,
    });
    frame.quad(crate::render::wgpu_renderer::GpuInstance {
        rect: [layout.content_x, h - reserve, layout.content_w, 1.0],
        color: color::alpha(color::STONE, 0.12),
        user: 0,
    });

    let font_px = typography::tier_at_most(reserve * 0.42, h);
    push_text(
        texts,
        [
            layout.content_x,
            y,
            layout.content_w,
            text_line_h(font_px),
        ],
        footer_hint_text(input_mode),
        font_px,
        color::alpha(color::UMBER, 0.72),
        false,
        TextAlign::Center,
    );
}

fn footer_hint_text(input_mode: InputMode) -> &'static str {
    match input_mode {
        InputMode::Keyboard => "ESC back   Enter select   Arrows navigate   Z / X view",
        InputMode::Controller => "B back   A select   D-pad navigate   LB / RB view",
        InputMode::Cursor => "Right-click back   Click select   Arrows navigate   Z / X view",
    }
}
