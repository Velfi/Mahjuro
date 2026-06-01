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
//! | `Manzu`, `manzu`, `characters` | Manzu suit red | Numbered suit |
//! | `Souzu`, `souzu`, `bamboos`, `bamboo` | Souzu green | Numbered suit |
//! | `Pinzu`, `pinzu`, `dots` | Pinzu suit blue | Numbered suit |
//! | `Winds`, `winds` | Wind gold | Honor suit family |
//! | `Dragons`, `dragons` | Red dragon (Chun) | Honor suit family |
//! | `Flowers`, `flowers` | Flower pink | Bonus tiles |
//! | `Seasons`, `seasons` | Season teal | Bonus tiles (solitaire) |
//! | `Honors`, `honors` | `color::keyword::HONORS` | Winds + dragons umbrella |
//! | `Chips`, `chips` | `color::keyword::CHIPS` | Score rail (cool) |
//! | `Mult`, `mult` | `color::keyword::MULT` | Score rail (warm) |
//! | `Gold`, `gold` | `color::keyword::GOLD` | Currency |
//! | `Play`, `play` | `color::keyword::PLAY` | Bank-meld HUD verb |
//! | `Trigger`, `trigger` | `color::keyword::TRIGGER` | Cash-in HUD verb |
//!
//! 3D cascade labels and streaming score popups reuse `LAPIS` / `RUBY` /
//! `RELIC_GOLD` / `PARCHMENT` for chips, mult, gold, and final totals — see
//! [`crate::render::score_popups`] and [`crate::scenes::gameplay::cascade_hud`].
//!
//! For paragraphs that use **safe inline markup** (`**`, `{{effect:…}}`, …)
//! together with the same per-word tinting, use [`crate::ui::widget::push_text_block`]
//! (`glossary_tint: true`) instead of this module’s plain-text wrappers.

use crate::render::decal::load_ui_font;
#[allow(unused_imports)] // Re-exported for API parity; table lives in `vocabulary_colors`.
pub use crate::render::vocabulary_colors::COLORED_KEYWORD_TABLE;
#[allow(unused_imports)]
// Re-exported for API parity; implementation lives in `vocabulary_colors`.
pub use crate::render::vocabulary_colors::color_for_token;
pub use crate::render::vocabulary_colors::colored_token_segments;
use crate::render::wgpu_renderer::{TextAlign, TextLabel};
use crate::ui::text_wrap::{TextBreakUnit, break_units_kp};
use crate::ui::widget;

/// Vertical step between colored keyword rows ([`push_colored_rows_left`], tooltips, guide panels).
/// All measure helpers and push paths must use this — do not duplicate the multiplier elsewhere.
pub const COLORED_ROW_LINE_STEP_MUL: f32 = 1.4;

#[inline]
pub fn colored_row_line_step(line_h: f32) -> f32 {
    line_h * COLORED_ROW_LINE_STEP_MUL
}

/// Height of already-wrapped rows at `line_h`.
pub fn colored_wrapped_rows_height(rows: &[Vec<(String, [f32; 4])>], line_h: f32) -> f32 {
    colored_row_line_step(line_h) * rows.len().max(1) as f32
}

/// Measure a single left-aligned colored line (same wrap + step as [`push_colored_line_left`]).
pub fn colored_line_block_height(text: &str, inner_w: f32, line_h: f32, default: [f32; 4]) -> f32 {
    let wrapped = wrap_colored_words(text, inner_w, line_h, default);
    colored_wrapped_rows_height(&wrapped, line_h)
}

/// Height of [`wrap_colored_text_multiline`] output (focus inspect, `\n` in tooltips).
pub fn colored_multiline_text_height(
    text: &str,
    inner_w: f32,
    line_h: f32,
    default: [f32; 4],
) -> f32 {
    let lines = wrap_colored_text_multiline(text, inner_w, line_h, default);
    colored_wrapped_rows_height(&lines, line_h)
}

/// Sum of [`colored_line_block_height`] for multiple single-line strings.
pub fn colored_lines_block_height(
    lines: &[&str],
    inner_w: f32,
    line_h: f32,
    default: [f32; 4],
) -> f32 {
    lines
        .iter()
        .map(|line| colored_line_block_height(line, inner_w, line_h, default))
        .sum()
}

/// Pre-measured colored line — use [`Self::measure`] then [`Self::push_left`] so layout cannot drift.
pub struct ColoredLineBlock {
    wrapped: Vec<Vec<(String, [f32; 4])>>,
    line_h: f32,
}

impl ColoredLineBlock {
    pub fn measure(text: &str, inner_w: f32, line_h: f32, default: [f32; 4]) -> Self {
        Self {
            wrapped: wrap_colored_words(text, inner_w, line_h, default),
            line_h,
        }
    }

    pub fn height(&self) -> f32 {
        colored_wrapped_rows_height(&self.wrapped, self.line_h)
    }

    pub fn push_left(
        &self,
        out: &mut Vec<TextLabel>,
        text_left: f32,
        top_y: f32,
        inner_w: f32,
        fallback_plain: &str,
        fallback_color: [f32; 4],
    ) {
        push_colored_rows_left(
            out,
            ColoredRowsLayout {
                text_left,
                top_y,
                inner_w,
                line_h: self.line_h,
                fallback_plain,
                fallback_color,
            },
            &self.wrapped,
        );
    }
}

/// Wrap, push, and return drawn height (always equals [`colored_line_block_height`] for the same inputs).
pub fn push_colored_line_left(
    out: &mut Vec<TextLabel>,
    text_left: f32,
    top_y: f32,
    inner_w: f32,
    line_h: f32,
    text: &str,
    default: [f32; 4],
) -> f32 {
    let block = ColoredLineBlock::measure(text, inner_w, line_h, default);
    let h = block.height();
    block.push_left(out, text_left, top_y, inner_w, text, default);
    h
}

fn word_width(font: &fontdue::Font, word: &str, font_px: f32) -> f32 {
    word.chars()
        .map(|ch| font.metrics(ch, font_px).advance_width)
        .sum()
}

/// Single-line advance width for a colored paragraph at `line_h`, capped by `max_width_px`.
/// Used to size tooltip panels to their copy instead of a fixed fraction of the window.
pub fn colored_paragraph_preferred_width(text: &str, line_h: f32, max_width_px: f32) -> f32 {
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    let Some(font) = load_ui_font() else {
        return (text.chars().count() as f32 * line_h * 0.55).min(max_width_px);
    };
    let font_px = line_h * 0.99;
    let space_w = font.metrics(' ', font_px).advance_width;
    let default = [0.0; 4];
    let mut widest = 0.0_f32;
    for paragraph in text.split('\n') {
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let mut line_w = 0.0;
        for (i, word) in words.iter().enumerate() {
            if i > 0 {
                line_w += space_w;
            }
            for (seg, _) in colored_token_segments(word, default) {
                line_w += word_width(font, &seg, font_px);
            }
        }
        widest = widest.max(line_w);
    }
    widest.clamp(0.0, max_width_px)
}

/// Line-wrap `text` into rows of `(word, color)` chunks (spaces omitted as
/// separate chunks; a following space is implied between words except at
/// line breaks). Words are atomic — punctuation split for tinting never
/// becomes its own line — and breaks are chosen with TeX-style demerits
/// ([`crate::ui::text_wrap::break_units_kp`]).
pub fn wrap_colored_words(
    text: &str,
    max_width_px: f32,
    line_h: f32,
    default: [f32; 4],
) -> Vec<Vec<(String, [f32; 4])>> {
    let Some(font) = load_ui_font() else {
        return vec![vec![(text.to_string(), default)]];
    };
    let font_px = line_h * 0.99;
    let space_w = font.metrics(' ', font_px).advance_width;

    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        if text.trim().is_empty() {
            return vec![];
        }
        return vec![vec![(text.to_string(), default)]];
    }

    let units: Vec<TextBreakUnit<Vec<(String, [f32; 4])>>> = words
        .iter()
        .map(|w| {
            let segments = colored_token_segments(w, default);
            let width = segments
                .iter()
                .map(|(seg, _)| word_width(font, seg, font_px))
                .sum();
            TextBreakUnit {
                width,
                payload: segments,
            }
        })
        .collect();

    let broken = break_units_kp(&units, max_width_px, space_w);
    broken
        .into_iter()
        .map(|word_segments| {
            let mut line: Vec<(String, [f32; 4])> = Vec::new();
            for (wi, segments) in word_segments.into_iter().enumerate() {
                if wi > 0 {
                    line.push((" ".to_string(), default));
                }
                line.extend(segments);
            }
            line
        })
        .collect()
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
        out.extend(wrap_colored_words(paragraph, max_width_px, line_h, default));
    }
    if out.is_empty() {
        out.push(vec![(String::new(), default)]);
    }
    out
}

/// Total height for [`wrap_colored_text_multiline`] at `line_h`.
pub fn colored_multiline_block_height(line_count: usize, line_h: f32) -> f32 {
    colored_row_line_step(line_h) * line_count as f32
}

/// Layout for [`push_colored_rows_left`] / [`push_colored_rows_in_width`].
pub struct ColoredRowsLayout<'a> {
    pub text_left: f32,
    pub top_y: f32,
    pub inner_w: f32,
    pub line_h: f32,
    pub fallback_plain: &'a str,
    pub fallback_color: [f32; 4],
}

/// Left-aligned rows (focus inspect, shop tooltips).
pub fn push_colored_rows_left(
    out: &mut Vec<TextLabel>,
    layout: ColoredRowsLayout<'_>,
    lines: &[Vec<(String, [f32; 4])>],
) {
    let ColoredRowsLayout {
        text_left,
        top_y,
        inner_w,
        line_h,
        fallback_plain,
        fallback_color,
    } = layout;
    let font_px = line_h;
    let line_step = colored_row_line_step(line_h);
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
            ..Default::default()
        });
        return;
    };

    for (row, chunks) in lines.iter().enumerate() {
        let line_y = top_y + row as f32 * line_step;
        let mut cx = text_left;
        for (s, c) in chunks {
            let piece_w = word_width(font, s, font_px).max(1.0);
            out.push(TextLabel {
                rect: [cx, line_y, piece_w, line_step],
                text: s.clone(),
                color: *c,
                font_px: Some(font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
            cx += piece_w;
        }
    }
}

/// Horizontally aligned colored rows inside `[block_left, top_y, inner_w, …]`.
pub fn push_colored_rows_in_width(
    out: &mut Vec<TextLabel>,
    layout: ColoredRowsLayout<'_>,
    lines: &[Vec<(String, [f32; 4])>],
    align: TextAlign,
) {
    let ColoredRowsLayout {
        text_left: block_left,
        top_y,
        inner_w,
        line_h,
        fallback_plain,
        fallback_color,
    } = layout;
    let font_px = line_h;
    let line_step = colored_row_line_step(line_h);
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
            ..Default::default()
        });
        return;
    };

    for (row, chunks) in lines.iter().enumerate() {
        let line_y = top_y + row as f32 * line_step;
        let measured: f32 = chunks
            .iter()
            .map(|(s, _)| word_width(font, s, font_px))
            .sum();
        let line_start = match align {
            TextAlign::Left => block_left,
            TextAlign::Center => block_left + (inner_w - measured) * 0.5,
            TextAlign::Right => block_left + inner_w - measured,
        };
        let mut cx = line_start;
        for (s, c) in chunks {
            let piece_w = word_width(font, s, font_px).max(1.0);
            out.push(TextLabel {
                rect: [cx, line_y, piece_w, line_step],
                text: s.clone(),
                color: *c,
                font_px: Some(font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
            cx += piece_w;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::theme::color;

    #[test]
    fn push_colored_line_left_matches_colored_line_block_height() {
        let text = "Manzu — Characters, ranks 1–9.";
        let inner_w = 420.0;
        let line_h = 22.0;
        let default = color::PARCHMENT;
        let measured = colored_line_block_height(text, inner_w, line_h, default);
        let mut labels = Vec::new();
        let drawn = push_colored_line_left(&mut labels, 0.0, 0.0, inner_w, line_h, text, default);
        assert!(
            (measured - drawn).abs() < 0.01,
            "measure/draw height drift: measured={measured} drawn={drawn}"
        );
    }

    #[test]
    fn colored_line_block_height_uses_row_step_multiplier() {
        let line_h = 20.0;
        let h = colored_line_block_height("foo bar", 800.0, line_h, [1.0; 4]);
        assert!((h - colored_row_line_step(line_h)).abs() < 0.01);
    }

    #[test]
    fn colored_wrap_keeps_trailing_punct_with_word() {
        let font_px = 28.0;
        let line_h = font_px / 0.99;
        let text = "An East Wind is blowing!";
        let font = load_ui_font().expect("ui font");
        let max_w = word_width(font, "An East Wind is blowing", font_px) + 2.0;
        let default = [1.0, 1.0, 1.0, 1.0];
        let lines = wrap_colored_words(text, max_w, line_h, default);
        let joined: Vec<String> = lines
            .iter()
            .map(|row| row.iter().map(|(s, _)| s.as_str()).collect::<String>())
            .collect();
        assert!(
            joined.iter().any(|l| l.contains('!')),
            "expected ! on same line as blowing, got {joined:?}"
        );
        assert!(
            !joined.iter().any(|l| l.trim() == "!"),
            "orphan ! line: {joined:?}"
        );
    }
}
