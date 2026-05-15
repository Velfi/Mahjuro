//! Word-wrapped UI paragraphs with a canonical **tinted vocabulary**.
//!
//! # Colored keyword reference
//!
//! Whitespace splits the paragraph into tokens; trailing punctuation
//! `, . ; : ! ? ' "` and closing quotes/brackets are stripped before lookup.
//! Match is ASCII case-insensitive. **Longest** `needle` in
//! [`crate::render::vocabulary_colors::COLORED_KEYWORD_TABLE`] wins.
//!
//! | Needle(s) | Theme | Meaning |
//! | --- | --- | --- |
//! | `Characters`, `characters` | Character suit red | Numbered suit |
//! | `Bamboos`, `Bamboo`, `bamboo` | Bamboo green | Numbered suit |
//! | `Dots`, `dots` | Dots suit blue | Numbered suit |
//! | `Winds`, `winds` | Wind gold | Honor suit family |
//! | `Dragons`, `dragons` | Red dragon (Chun) | Honor suit family |
//! | `Flowers`, `flowers` | Flower pink | Bonus tiles |
//! | `Seasons`, `seasons` | Season teal | Bonus tiles (solitaire) |
//! | `Honors`, `honors` | `CHAMPAGNE` | Winds + dragons umbrella |
//! | `Chips`, `chips` | `LAPIS` | Score rail (cool) |
//! | `Mult`, `mult` | `RUBY` | Score rail (warm) |
//! | `Gold`, `gold` | `RELIC_GOLD` | Currency |
//! | `Play`, `play` | `LEAF_GREEN` | Bank-meld HUD verb |
//! | `Trigger`, `trigger` | `BRASS` | Cash-in HUD verb |
//!
//! 3D cascade labels and streaming score popups reuse `LAPIS` / `RUBY` /
//! `RELIC_GOLD` / `TALLOW` for chips, mult, gold, and final totals — see
//! [`crate::render::score_popups`] and [`crate::scenes::gameplay::cascade_hud`].
//!
//! For paragraphs that use **safe inline markup** (`**`, `{{effect:…}}`, …)
//! together with the same per-word tinting, use [`crate::ui::widget::push_text_block`]
//! (`glossary_tint: true`) instead of this module’s plain-text wrappers.

use crate::render::decal::load_ui_font;
#[allow(unused_imports)] // Re-exported for API parity; implementation lives in `vocabulary_colors`.
pub use crate::render::vocabulary_colors::color_for_token;
pub use crate::render::vocabulary_colors::colored_token_segments;
#[allow(unused_imports)] // Re-exported for API parity; table lives in `vocabulary_colors`.
pub use crate::render::vocabulary_colors::COLORED_KEYWORD_TABLE;
use crate::render::wgpu_renderer::{TextAlign, TextLabel};
use crate::ui::widget::{self, TextStyle};

fn word_width(font: &fontdue::Font, word: &str, font_px: f32) -> f32 {
    word.chars()
        .map(|ch| font.metrics(ch, font_px).advance_width)
        .sum()
}

/// Line-wrap `text` into rows of `(word, color)` chunks (spaces omitted as
/// separate chunks; a following space is implied between words except at
/// line breaks). Uses the same greedy width heuristic as [`crate::ui::widget::wrap_text`].
pub fn wrap_colored_words(
    text: &str,
    max_width_px: f32,
    line_h: f32,
    default: [f32; 4],
) -> Vec<Vec<(String, [f32; 4])>> {
    let Some(font) = load_ui_font() else {
        return vec![vec![(text.to_string(), default)]];
    };
    let font_px = (line_h * 0.99).max(8.0);
    let space_w = font.metrics(' ', font_px).advance_width;

    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        if text.trim().is_empty() {
            return vec![];
        }
        return vec![vec![(text.to_string(), default)]];
    }

    let mut lines: Vec<Vec<(String, [f32; 4])>> = Vec::new();
    let mut current: Vec<(String, [f32; 4])> = Vec::new();
    let mut line_w = 0.0_f32;

    for w in words.iter() {
        let segments = colored_token_segments(w, default);
        for (si, (seg, col)) in segments.iter().enumerate() {
            let w_w = word_width(&font, seg, font_px);
            if w_w > max_width_px {
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                }
                lines.push(vec![(seg.clone(), *col)]);
                line_w = 0.0;
                continue;
            }
            let needs_word_gap = si == 0 && !current.is_empty();
            let extra = if current.is_empty() {
                w_w
            } else if needs_word_gap {
                space_w + w_w
            } else {
                w_w
            };
            if !current.is_empty() && line_w + extra > max_width_px {
                lines.push(std::mem::take(&mut current));
                line_w = 0.0;
            }
            if needs_word_gap {
                current.push((" ".to_string(), default));
                line_w += space_w;
            }
            current.push((seg.clone(), *col));
            line_w += w_w;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Same line breaks as [`widget::wrap_text`] (explicit `\n` splits paragraphs),
/// with per-word keyword tinting.
pub fn wrap_colored_text_multiline(
    text: &str,
    max_width_px: f32,
    line_h: f32,
    default: [f32; 4],
) -> Vec<Vec<(String, [f32; 4])>> {
    if load_ui_font().is_none() {
        return widget::wrap_text(text, max_width_px, line_h)
            .into_iter()
            .map(|s| vec![(s, default)])
            .collect();
    }
    let mut out: Vec<Vec<(String, [f32; 4])>> = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(vec![(String::new(), default)]);
            continue;
        }
        out.extend(wrap_colored_words(
            paragraph,
            max_width_px,
            line_h,
            default,
        ));
    }
    if out.is_empty() {
        out.push(vec![(String::new(), default)]);
    }
    out
}

/// Total height for [`wrap_colored_text_multiline`] at `line_h` (same `1.4` step as tooltips).
pub fn colored_multiline_block_height(line_count: usize, line_h: f32) -> f32 {
    let line_step = line_h * 1.4;
    line_count as f32 * line_step
}

/// Left-aligned rows (focus inspect, shop tooltips).
pub fn push_colored_rows_left(
    out: &mut Vec<TextLabel>,
    text_left: f32,
    top_y: f32,
    inner_w: f32,
    lines: &[Vec<(String, [f32; 4])>],
    line_h: f32,
    fallback_plain: &str,
    fallback_color: [f32; 4],
) {
    let font_px = line_h.max(8.0);
    let line_step = line_h * 1.4;
    let Some(font) = load_ui_font() else {
        let wrapped = widget::wrap_text(fallback_plain, inner_w, line_h);
        let joined = wrapped.join("\n");
        let h = colored_multiline_block_height(wrapped.len().max(1), line_h);
        out.push(TextLabel {
            rect: [text_left, top_y, inner_w, h],
            text: joined,
            color: fallback_color,
            font_px: Some(font_px),
            align: TextAlign::Left,
            no_glossary: true,
            ..Default::default()
        });
        return;
    };

    for (row, chunks) in lines.iter().enumerate() {
        let line_y = top_y + row as f32 * line_step;
        let mut cx = text_left;
        for (s, c) in chunks {
            let piece_w = word_width(&font, s, font_px).max(1.0);
            out.push(TextLabel {
                rect: [cx, line_y, piece_w, line_step],
                text: s.clone(),
                color: *c,
                font_px: Some(font_px),
                align: TextAlign::Left,
                no_glossary: true,
                ..Default::default()
            });
            cx += piece_w;
        }
    }
}

/// Horizontally aligned colored rows inside `[block_left, top_y, inner_w, …]`.
pub fn push_colored_rows_in_width(
    out: &mut Vec<TextLabel>,
    block_left: f32,
    top_y: f32,
    inner_w: f32,
    lines: &[Vec<(String, [f32; 4])>],
    line_h: f32,
    align: TextAlign,
    fallback_plain: &str,
    fallback_color: [f32; 4],
) {
    let font_px = line_h.max(8.0);
    let line_step = line_h * 1.4;
    let Some(font) = load_ui_font() else {
        let wrapped = widget::wrap_text(fallback_plain, inner_w, line_h);
        let joined = wrapped.join("\n");
        let h = colored_multiline_block_height(wrapped.len().max(1), line_h);
        out.push(TextLabel {
            rect: [block_left, top_y, inner_w, h],
            text: joined,
            color: fallback_color,
            font_px: Some(font_px),
            align,
            no_glossary: true,
            ..Default::default()
        });
        return;
    };

    for (row, chunks) in lines.iter().enumerate() {
        let line_y = top_y + row as f32 * line_step;
        let measured: f32 = chunks
            .iter()
            .map(|(s, _)| word_width(&font, s, font_px))
            .sum();
        let line_start = match align {
            TextAlign::Left => block_left,
            TextAlign::Center => block_left + (inner_w - measured) * 0.5,
            TextAlign::Right => block_left + inner_w - measured,
        };
        let mut cx = line_start;
        for (s, c) in chunks {
            let piece_w = word_width(&font, s, font_px).max(1.0);
            out.push(TextLabel {
                rect: [cx, line_y, piece_w, line_step],
                text: s.clone(),
                color: *c,
                font_px: Some(font_px),
                align: TextAlign::Left,
                no_glossary: true,
                ..Default::default()
            });
            cx += piece_w;
        }
    }
}

pub fn colored_wrapped_line_count(
    text: &str,
    max_width_px: f32,
    line_h: f32,
    default: [f32; 4],
) -> usize {
    let lines = wrap_colored_text_multiline(text, max_width_px, line_h, default);
    if lines.is_empty() {
        return 1;
    }
    lines.len()
}

/// Push one paragraph as multiple [`TextLabel`]s (one per colored chunk).
pub fn push_colored_text_block(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    text: &str,
    style: TextStyle,
    window_h: f32,
) {
    let [x, y, w, h] = rect;
    let pad = style.padding;
    let inner_w = (w - 2.0 * pad).max(1.0);
    let inner_h = (h - 2.0 * pad).max(1.0);
    let line_h = crate::render::theme::typography::size(style.tier, window_h);
    let font_px = line_h.max(8.0);
    let line_step = line_h * 1.4;
    let max_lines = ((inner_h / line_step).floor() as usize).max(1);

    let lines = wrap_colored_words(text, inner_w, line_h, style.color);
    let lines: Vec<_> = lines.into_iter().take(max_lines).collect();
    if lines.is_empty() {
        return;
    }
    let n = lines.len();
    let total_h = line_step * n as f32;
    let block_top = y + pad + ((inner_h - total_h) * 0.5).max(0.0);

    let Some(font) = load_ui_font() else {
        out.push(TextLabel {
            rect: [x + pad, y + pad, inner_w, inner_h],
            text: text.to_string(),
            color: style.color,
            font_px: Some(font_px),
            align: style.align,
            ..Default::default()
        });
        return;
    };

    for (row, chunks) in lines.iter().enumerate() {
        let line_y = block_top + row as f32 * line_step;
        let mut measured = 0.0_f32;
        for (s, _) in chunks {
            measured += word_width(&font, s, font_px);
        }
        let line_start = match style.align {
            TextAlign::Left => x + pad,
            TextAlign::Center => x + pad + (inner_w - measured) * 0.5,
            TextAlign::Right => x + pad + inner_w - measured,
        };
        let mut cx = line_start;
        for (s, c) in chunks {
            let piece_w = word_width(&font, s, font_px).max(1.0);
            out.push(TextLabel {
                rect: [cx, line_y, piece_w, line_step],
                text: s.clone(),
                color: *c,
                font_px: Some(font_px),
                align: TextAlign::Left,
                no_glossary: true,
                ..Default::default()
            });
            cx += piece_w;
        }
    }
}
