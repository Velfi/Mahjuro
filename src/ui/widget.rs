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

use crate::render::decal::load_ui_font;
use crate::render::theme::{self, ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::scenes::ButtonDef;
use crate::ui::input::UiAction;

/// Same as calling [`push_panel_colored`] but with explicit colors. Used by score panel and
/// shop cards which need fine-grained control over the gold flash overlays.
pub fn push_panel_colored(
    out: &mut Vec<GpuInstance>,
    rect: [f32; 4],
    bg: [f32; 4],
    border: [f32; 4],
) {
    // Background fill.
    out.push(GpuInstance { rect, color: bg });
    let bt = border_thickness(rect);
    push_inset_border(out, rect, border, bt);
    // Bevel: one pixel of highlight on the inner top/left edges, shadow on
    // inner bottom/right edges. Sits just inside the inset border so it reads
    // as a raised-panel chamfer rather than a second border.
    push_bevel(out, rect, bt);
}

/// Standard border thickness for a rect — small enough to look like an
/// inlay, large enough to be visible at low resolutions.
fn border_thickness(rect: [f32; 4]) -> f32 {
    (rect[3] * 0.018).clamp(1.0, 2.0)
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

/// Draw a one-pixel raised-panel bevel just inside the inset border.
///
/// The top and left inner edges get a subtle highlight (lighter indigo) and
/// the bottom and right inner edges get a shadow (near-black), making the
/// panel read as physically raised out of the background.
///
/// `border_t` is the thickness of the already-drawn inset border so the bevel
/// starts at the correct inset position.
fn push_bevel(out: &mut Vec<GpuInstance>, rect: [f32; 4], border_t: f32) {
    use color::*;
    let [x, y, w, h] = rect;
    let o = border_t; // offset: bevel sits just inside the border
    let bw = 1.0_f32; // bevel strip width (1 px looks crisp at all scales)
    let hi = alpha(TWILIGHT, 0.55); // top-left highlight — lighter indigo
    let sh = alpha(OBSIDIAN, 0.70); // bottom-right shadow — near-black

    // Highlight: top inner edge
    out.push(GpuInstance {
        rect: [x + o, y + o, w - 2.0 * o, bw],
        color: hi,
    });
    // Highlight: left inner edge (skip the top-left corner already covered)
    out.push(GpuInstance {
        rect: [x + o, y + o + bw, bw, h - 2.0 * o - bw],
        color: hi,
    });

    // Shadow: bottom inner edge
    out.push(GpuInstance {
        rect: [x + o, y + h - o - bw, w - 2.0 * o, bw],
        color: sh,
    });
    // Shadow: right inner edge (skip the bottom-right corner)
    out.push(GpuInstance {
        rect: [x + w - o - bw, y + o, bw, h - 2.0 * o - bw],
        color: sh,
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
    buttons.push(ButtonDef::ui((rect[0], rect[1], rect[2], rect[3]), action));
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
        ..Default::default()
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
    /// Horizontal alignment for each wrapped line.
    pub align: TextAlign,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            tier: typography::BODY,
            color: color::PARCHMENT,
            padding: 0.0,
            align: TextAlign::Center,
        }
    }
}

/// Wrap `text` into multiple lines that fit `rect` (minus padding) and push
/// a single multi-line `TextLabel`. The label carries an explicit `font_px`
/// so every line in the paragraph rasterises at exactly the same size — no
/// per-line auto-shrink, no jagged sizing across the block.
///
/// This is the helper that ensures long descriptions don't get crammed into
/// raw slot rects — the explicit fix for prior text-readability feedback.
pub fn push_text_block(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    text: &str,
    style: TextStyle,
    window_h: f32,
    ui_scale: f32,
) {
    let [x, y, w, h] = rect;
    let pad = style.padding;
    let inner_w = (w - 2.0 * pad).max(1.0);
    let inner_h = (h - 2.0 * pad).max(1.0);
    let line_h = typography::size(style.tier, window_h, ui_scale);
    // The rasteriser pins font_px directly, so it doesn't depend on the
    // rect's aspect ratio anymore. Use line_h as the pinned font_px.
    let font_px = line_h.max(8.0);
    let line_step = line_h * 1.4;
    let max_lines = ((inner_h / line_step).floor() as usize).max(1);

    let lines = wrap_text(text, inner_w, line_h);
    let drawn: Vec<&String> = lines.iter().take(max_lines).collect();
    let joined = drawn
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<&str>>()
        .join("\n");

    out.push(TextLabel {
        rect: [x + pad, y + pad, inner_w, inner_h],
        text: joined,
        color: style.color,
        font_px: Some(font_px),
        align: style.align,
        ..Default::default()
    });
}

/// Greedy word-wrap at a fixed target font size.
///
/// Important: do *not* go through `measure_label_advances` here. That helper
/// auto-shrinks `font_px` to make over-long text fit a rect, so a long string
/// would always "fit" `max_width_px` at a tiny font and the wrapper would
/// never break a line. Instead we measure each word's advance width at the
/// font size the line will actually render at — `font_px ≈ line_h * 0.99`,
/// matching `rasterize_label`'s `height * 0.55` term against the line-box
/// height of `line_h * 1.8` that `push_text_block` produces.
pub fn wrap_text(text: &str, max_width_px: f32, line_h: f32) -> Vec<String> {
    let Some(font) = load_ui_font() else {
        // No font loaded — don't crash, just return the input as one line.
        return vec![text.to_string()];
    };
    let font_px = (line_h * 0.99).max(8.0);
    let space_w = font.metrics(' ', font_px).advance_width;
    let word_w = |w: &str| -> f32 {
        w.chars()
            .map(|c| font.metrics(c, font_px).advance_width)
            .sum()
    };

    let mut lines: Vec<String> = Vec::new();

    // Process each explicit line separately so '\n' always forces a break.
    for paragraph in text.split('\n') {
        let mut current = String::new();
        let mut current_w = 0.0f32;

        for word in paragraph.split_whitespace() {
            let ww = word_w(word);
            let need = if current.is_empty() { ww } else { space_w + ww };
            if !current.is_empty() && current_w + need > max_width_px {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
                current_w = ww;
            } else {
                if !current.is_empty() {
                    current.push(' ');
                    current_w += space_w;
                }
                current.push_str(word);
                current_w += ww;
            }
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Push a small "price tag" pill — used by the shop. Brass background with
/// champagne numerals; desaturates when `affordable` is false.
#[allow(dead_code)]
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
        ..Default::default()
    });
}
