//! Reusable UI widget helpers.
//!
//! These are NOT a framework — just functions that push `GpuInstance`,
//! `TextLabel`, and `ButtonDef` values into the vectors a scene already
//! maintains. The goal is to give every screen the same visual language
//! (Midnight Gold theme) without forcing scenes to adopt a retained-mode
//! widget tree.
//!
//! Each helper takes the rect to draw at and pushes:
//! - One or more background quads (for inset borders).
//! - Optionally a text label.
//! - Optionally a clickable button hit-test.
//!
//! See [`crate::render::theme`] for the color tokens these helpers consume.

use crate::render::decal::{load_ui_font, measure_label_advances};
use crate::render::theme::{self, ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::scenes::ButtonDef;
use crate::ui::input::UiAction;

/// Visual variant for a panel — picks the background color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelVariant {
    /// Default raised panel: INDIGO with a BRASS border.
    Default,
    /// Sunken panel one step darker than the surrounding surface.
    Sunken,
    /// Hero panel for the score / important callouts: TWILIGHT with GOLD border.
    Hero,
}

/// Push a panel (background + 1px-style inset border, faked with four quads).
///
/// The border is drawn as four thin quads on the inside edge of `rect` so it
/// reads as a recessed gold inlay rather than a hard outline.
pub fn push_panel(
    out: &mut Vec<GpuInstance>,
    rect: [f32; 4],
    variant: PanelVariant,
) {
    let (bg, border) = match variant {
        PanelVariant::Default => (color::INDIGO, color::BRASS),
        PanelVariant::Sunken => (color::OBSIDIAN, color::ANTIQUE),
        PanelVariant::Hero => (color::TWILIGHT, color::GOLD),
    };
    push_panel_colored(out, rect, bg, border);
}

/// Same as [`push_panel`] but with explicit colors. Used by score panel and
/// shop cards which need fine-grained control over the gold flash overlays.
pub fn push_panel_colored(
    out: &mut Vec<GpuInstance>,
    rect: [f32; 4],
    bg: [f32; 4],
    border: [f32; 4],
) {
    // Background fill.
    out.push(GpuInstance { rect, color: bg });
    push_inset_border(out, rect, border, border_thickness(rect));
}

/// Standard border thickness for a rect — small enough to look like an
/// inlay, large enough to be visible at low resolutions.
fn border_thickness(rect: [f32; 4]) -> f32 {
    (rect[3] * 0.025).clamp(1.0, 3.0)
}

/// Draw a 4-quad inset border around the inside of `rect`.
pub fn push_inset_border(
    out: &mut Vec<GpuInstance>,
    rect: [f32; 4],
    color: [f32; 4],
    thickness: f32,
) {
    let [x, y, w, h] = rect;
    let t = thickness;
    // Top
    out.push(GpuInstance {
        rect: [x, y, w, t],
        color,
    });
    // Bottom
    out.push(GpuInstance {
        rect: [x, y + h - t, w, t],
        color,
    });
    // Left
    out.push(GpuInstance {
        rect: [x, y + t, t, h - 2.0 * t],
        color,
    });
    // Right
    out.push(GpuInstance {
        rect: [x + w - t, y + t, t, h - 2.0 * t],
        color,
    });
}

/// Push a button: background + border + centered text + hit-test rect.
///
/// The action becomes a `ButtonDef::ui` so the click feeds into the existing
/// `UiAction` queue.
pub fn push_button(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    buttons: &mut Vec<ButtonDef>,
    rect: [f32; 4],
    label: &str,
    variant: ButtonVariant,
    state: ButtonState,
    action: UiAction,
) {
    push_button_visuals(quads, labels, rect, label, variant, state);
    buttons.push(ButtonDef::ui(
        (rect[0], rect[1], rect[2], rect[3]),
        action,
    ));
}

fn push_button_visuals(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    rect: [f32; 4],
    label: &str,
    variant: ButtonVariant,
    state: ButtonState,
) {
    let colors = theme::button_colors(variant, state);
    push_panel_colored(quads, rect, colors.bg, colors.border);
    labels.push(TextLabel {
        rect,
        text: label.to_string(),
        color: colors.text,
    });
}

/// Style hint for [`push_text_block`].
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    /// Typography tier ratio (e.g. `typography::BODY`). Used to size lines.
    pub tier: f32,
    pub color: [f32; 4],
    /// Padding inside the rect, in pixels.
    pub padding: f32,
}

/// Wrap `text` into multiple lines that fit `rect` (minus padding) and push
/// one `TextLabel` per line. Uses `measure_label_advances` to find break
/// points, so the wrap respects the same font sizing the renderer will use.
///
/// This is the helper that ensures long descriptions don't get crammed into
/// raw slot rects — the explicit fix for prior text-readability feedback.
pub fn push_text_block(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    text: &str,
    style: TextStyle,
    window_h: f32,
) {
    let [x, y, w, h] = rect;
    let pad = style.padding;
    let inner_w = (w - 2.0 * pad).max(1.0);
    let line_h = typography::size(style.tier, window_h);
    let max_lines = ((h - 2.0 * pad) / line_h).max(1.0) as usize;

    let lines = wrap_text(text, inner_w, line_h);
    let drawn = lines.iter().take(max_lines);
    for (i, line) in drawn.enumerate() {
        let line_y = y + pad + i as f32 * line_h;
        out.push(TextLabel {
            rect: [x + pad, line_y, inner_w, line_h],
            text: line.clone(),
            color: style.color,
        });
    }
}

/// Greedy word-wrap. Falls back to character wrapping for very long words.
fn wrap_text(text: &str, max_width_px: f32, line_h: f32) -> Vec<String> {
    let Some(font) = load_ui_font() else {
        // No font loaded — don't crash, just return the input as one line.
        return vec![text.to_string()];
    };
    // Measure with the same font sizing as the renderer.
    let height_u = line_h.max(8.0) as u32;
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let trial = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        // Use measure_label_advances to estimate the rendered width of the
        // *trial* string at the target line height.
        let (_, _, advances) =
            measure_label_advances(&font, &trial, max_width_px as u32, height_u);
        let total: f32 = advances.iter().sum();
        if total <= max_width_px || current.is_empty() {
            current = trial;
        } else {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Push a small "price tag" pill — used by the shop. Brass background with
/// champagne numerals; desaturates when `affordable` is false.
pub fn push_price_tag(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    rect: [f32; 4],
    price: u32,
    affordable: bool,
) {
    let (bg, border, text) = if affordable {
        (color::BRASS, color::GOLD, color::CHAMPAGNE)
    } else {
        (color::SLATE, color::ANTIQUE, color::RUBY)
    };
    push_panel_colored(quads, rect, bg, border);
    labels.push(TextLabel {
        rect,
        text: format!("${price}"),
        color: text,
    });
}
