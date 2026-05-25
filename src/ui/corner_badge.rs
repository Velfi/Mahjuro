//! Small corner badges for flat UI overlays (shop FREE, archive NEW, …).

use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

/// Brass pill anchored to the top-right of `rect`.
pub fn push_corner_badge(
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
    rect: [f32; 4],
    window_h: f32,
    label: &str,
) {
    if rect[2] <= 1.0 || rect[3] <= 1.0 || !rect[0].is_finite() || !rect[1].is_finite() {
        return;
    }
    let badge_font = typography::size(typography::H45, window_h);
    let badge_h = (badge_font * 1.55).max(18.0);
    let char_w = badge_font * 0.52;
    let badge_w = (char_w * label.len() as f32 + badge_font * 1.2).max(36.0);
    let badge_x = rect[0] + rect[2] - badge_w * 0.88;
    let badge_y = rect[1] - badge_h * 0.22;

    quads.push(GpuInstance {
        rect: [badge_x, badge_y, badge_w, badge_h],
        color: [color::BRASS[0], color::BRASS[1], color::BRASS[2], 0.95],
        user: 0,
    });
    quads.push(GpuInstance {
        rect: [badge_x + 2.0, badge_y + 2.0, badge_w - 4.0, badge_h - 4.0],
        color: [
            color::WALNUT_DEEP[0],
            color::WALNUT_DEEP[1],
            color::WALNUT_DEEP[2],
            0.96,
        ],
        user: 0,
    });
    texts.push(TextLabel {
        rect: [badge_x + 4.0, badge_y, badge_w - 8.0, badge_h],
        text: label.to_string(),
        color: color::CHAMPAGNE,
        font_px: Some(badge_font),
        align: TextAlign::Center,
        ..Default::default()
    });
}

/// Small filled dot for tab / row unread hints.
pub fn push_new_dot(quads: &mut Vec<GpuInstance>, anchor_rect: [f32; 4], scale: f32) {
    let d = (10.0 * scale).clamp(8.0, 14.0);
    let x = anchor_rect[0] + anchor_rect[2] - d * 0.35;
    let y = anchor_rect[1] + d * 0.15;
    quads.push(GpuInstance {
        rect: [x, y, d, d],
        color: [color::BRASS[0], color::BRASS[1], color::BRASS[2], 0.98],
        user: 0,
    });
}
