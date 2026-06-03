//! Small corner badges for flat UI overlays (shop FREE, archive NEW, …).

use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

fn badge_metrics(label: &str, window_h: f32) -> (f32, f32, f32) {
    let badge_font = typography::size(typography::H45, window_h);
    let badge_h = (badge_font * 1.55).max(18.0);
    let char_w = badge_font * 0.52;
    let badge_w = (char_w * label.len() as f32 + badge_font * 1.2).max(36.0);
    (badge_w, badge_h, badge_font)
}

fn push_badge_at(
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
    badge_x: f32,
    badge_y: f32,
    badge_w: f32,
    badge_h: f32,
    badge_font: f32,
    label: &str,
) {
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

/// Screen rect `[x, y, w, h]` for a corner badge on `host`, clamped inside the viewport.
pub(crate) fn corner_badge_rect(
    host: [f32; 4],
    window_w: f32,
    window_h: f32,
    label: &str,
) -> Option<[f32; 4]> {
    if host[2] <= 1.0 || host[3] <= 1.0 || !host[0].is_finite() || !host[1].is_finite() {
        return None;
    }
    let (badge_w, badge_h, _) = badge_metrics(label, window_h);
    let badge_x = (host[0] + host[2] - badge_w * 0.88).clamp(0.0, (window_w - badge_w).max(0.0));
    let badge_y = (host[1] - badge_h * 0.22).clamp(0.0, (window_h - badge_h).max(0.0));
    Some([badge_x, badge_y, badge_w, badge_h])
}

/// Brass pill anchored to the top-right of `rect` (clamped to the viewport).
pub fn push_corner_badge(
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
    rect: [f32; 4],
    window_w: f32,
    window_h: f32,
    label: &str,
) {
    let Some([badge_x, badge_y, badge_w, badge_h]) =
        corner_badge_rect(rect, window_w, window_h, label)
    else {
        return;
    };
    let (_, _, badge_font) = badge_metrics(label, window_h);
    push_badge_at(
        quads, texts, badge_x, badge_y, badge_w, badge_h, badge_font, label,
    );
}

/// Brass pill centered on `rect` (archive cubby overlays).
///
/// When `occluder` is set, the badge is skipped if its screen rect intersects it (e.g. the
/// archive description sign, which is drawn in the room pass before cubby overlay quads).
pub fn push_center_badge(
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
    rect: [f32; 4],
    window_h: f32,
    label: &str,
    occluder: Option<[f32; 4]>,
) {
    if rect[2] <= 1.0 || rect[3] <= 1.0 || !rect[0].is_finite() || !rect[1].is_finite() {
        return;
    }
    let (badge_w, badge_h, badge_font) = badge_metrics(label, window_h);
    let badge_x = rect[0] + (rect[2] - badge_w) * 0.5;
    let badge_y = rect[1] + (rect[3] - badge_h) * 0.5;
    let badge_rect = [badge_x, badge_y, badge_w, badge_h];
    if occluder.is_some_and(|occ| crate::ui::clip::intersect_rect(badge_rect, occ).is_some()) {
        return;
    }
    push_badge_at(
        quads, texts, badge_x, badge_y, badge_w, badge_h, badge_font, label,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_badge_rect_clamps_past_right_edge() {
        let w = 800.0;
        let h = 600.0;
        let host = [720.0, 400.0, 90.0, 60.0];
        let badge = corner_badge_rect(host, w, h, "NEW").expect("badge");
        assert!(
            badge[0] + badge[2] <= w + 0.5,
            "badge should not extend past viewport"
        );
        assert!(badge[0] >= 0.0);
    }
}
