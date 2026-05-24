//! Reusable UI widget helpers.
//!
//! These are NOT a framework — just functions that push `GpuInstance`,
//! `TextLabel`, and `ButtonDef` values into the vectors a scene already
//! maintains. The goal is to give every screen the same visual language
//! (Walnut, Brass & Felt theme — see `COLOR_THEME.md`) without forcing
//! scenes to adopt a retained-mode widget tree.
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
    out.push(GpuInstance {
        rect,
        color: bg,
        user: 0,
    });
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
        user: 0,
    });
    // Bottom
    out.push(GpuInstance {
        rect: [x, y + h - t, w, t],
        color,
        user: 0,
    });
    // Left
    out.push(GpuInstance {
        rect: [x, y + t, t, h - 2.0 * t],
        color,
        user: 0,
    });
    // Right
    out.push(GpuInstance {
        rect: [x + w - t, y + t, t, h - 2.0 * t],
        color,
        user: 0,
    });
}

/// Draw a one-pixel raised-panel bevel just inside the inset border.
///
/// The top and left inner edges get a subtle highlight (lighter walnut) and
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
    let hi = alpha(WALNUT_BRIGHT, 0.55); // top-left highlight — lighter walnut
    let sh = alpha(WALNUT_INK, 0.70); // bottom-right shadow — near-black

    // Highlight: top inner edge
    out.push(GpuInstance {
        rect: [x + o, y + o, w - 2.0 * o, bw],
        color: hi,
        user: 0,
    });
    // Highlight: left inner edge (skip the top-left corner already covered)
    out.push(GpuInstance {
        rect: [x + o, y + o + bw, bw, h - 2.0 * o - bw],
        color: hi,
        user: 0,
    });

    // Shadow: bottom inner edge
    out.push(GpuInstance {
        rect: [x + o, y + h - o - bw, w - 2.0 * o, bw],
        color: sh,
        user: 0,
    });
    // Shadow: right inner edge (skip the bottom-right corner)
    out.push(GpuInstance {
        rect: [x + w - o - bw, y + o, bw, h - 2.0 * o - bw],
        color: sh,
        user: 0,
    });
}

/// A button: rect + label + visual style + action fired on click.
pub struct ButtonSpec<'a> {
    pub rect: [f32; 4],
    pub label: &'a str,
    pub variant: ButtonVariant,
    pub state: ButtonState,
    pub action: UiAction,
}

/// Push a button: background + border + centered text + hit-test rect.
///
/// The action becomes a `ButtonDef::ui` so the click feeds into the existing
/// `UiAction` queue.
pub fn push_button(
    quads: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    buttons: &mut Vec<ButtonDef>,
    spec: ButtonSpec<'_>,
) {
    let ButtonSpec {
        rect,
        label,
        variant,
        state,
        action,
    } = spec;
    push_button_visuals(quads, labels, rect, label, variant, state);
    buttons.push(
        ButtonDef::ui((rect[0], rect[1], rect[2], rect[3]), action)
            .with_hover_label(label.to_string()),
    );
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
    /// Typography tier ratio (e.g. `typography::H36`). Used to size lines.
    pub tier: f32,
    pub color: [f32; 4],
    /// Padding inside the rect, in pixels.
    pub padding: f32,
    /// Horizontal alignment for each wrapped line.
    pub align: TextAlign,
    /// Per-word vocabulary tint ([`crate::ui::colored_keywords::color_for_token`]).
    pub glossary_tint: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            tier: typography::H36,
            color: color::PARCHMENT,
            padding: 0.0,
            align: TextAlign::Center,
            glossary_tint: false,
        }
    }
}

/// Wrap `text` into multiple lines that fit `rect` (minus padding) and push
/// styled `TextLabel`s (safe inline markup + optional glossary tint).
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
    crate::ui::styled_text::push_styled_text_block(out, rect, text, style.into(), window_h);
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
/// Vertical step multiplier for [`wrap_text`] blocks (matches [`push_dense_text_lines`] in scenes).
pub const PLAIN_TEXT_LINE_STEP_MUL: f32 = 1.22;

/// Height of wrapped plain text at `font_px` and `line_mul` (same math as scene body copy).
pub fn plain_text_block_height(text: &str, max_width_px: f32, font_px: f32, line_mul: f32) -> f32 {
    let wrapped = wrap_text(text, max_width_px, font_px / 0.99);
    font_px * line_mul * wrapped.len().max(1) as f32
}

pub fn wrap_text(text: &str, max_width_px: f32, line_h: f32) -> Vec<String> {
    let Some(font) = load_ui_font() else {
        // No font loaded — don't crash, just return the input as one line.
        return vec![text.to_string()];
    };
    let font_px = line_h * 0.99;
    let space_w = font.metrics(' ', font_px).advance_width;
    let word_w = |w: &str| -> f32 {
        w.chars()
            .map(|c| font.metrics(c, font_px).advance_width)
            .sum()
    };

    let mut lines: Vec<String> = Vec::new();

    // Process each explicit line separately so '\n' always forces a break.
    for paragraph in text.split('\n') {
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            lines.push(String::new());
            continue;
        }
        lines.extend(crate::ui::text_wrap::wrap_words_kp(
            &words,
            word_w,
            max_width_px,
            space_w,
        ));
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
