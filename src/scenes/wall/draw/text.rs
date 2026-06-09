//! Shared text and plaque drawing helpers.

use crate::render::draw_cmd::UiFrame;
use crate::render::text_effect::TextEffectId;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

pub fn push_text(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    text: impl Into<String>,
    font_px: f32,
    color: [f32; 4],
    bold: bool,
    align: TextAlign,
) {
    push_text_maybe_clip(out, rect, text, font_px, color, bold, align, None);
}

pub fn push_text_maybe_clip(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    text: impl Into<String>,
    font_px: f32,
    color: [f32; 4],
    bold: bool,
    align: TextAlign,
    clip: Option<[f32; 4]>,
) {
    out.push(TextLabel {
        rect,
        text: text.into(),
        color,
        font_px: Some(font_px),
        align,
        scroll_offset: 0.0,
        flavor_spans: None,
        bold,
        italic: false,
        underline: false,
        text_effect: TextEffectId::Flat,
        rotation_quarters: 0,
        baseline_shift_px: 0.0,
        clip_rect: clip,
        block_vertical_align: Default::default(),
        mono: false,
    });
}

pub fn push_plaque(frame: &mut UiFrame, rect: [f32; 4], alpha: f32) {
    frame.quad(GpuInstance {
        rect,
        color: color::alpha(color::WALNUT_DEEP, alpha),
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1], rect[2], 1.0],
        color: color::alpha(color::STONE, 0.28),
        user: 0,
    });
}
