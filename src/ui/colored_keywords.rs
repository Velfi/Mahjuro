//! Word-wrapped UI paragraphs with a canonical **tinted vocabulary**.
//!
//! # Colored keyword reference
//!
//! Whitespace splits the paragraph into tokens; trailing punctuation
//! `, . ; : ! ? ' "` and closing quotes/brackets are stripped before lookup.
//! Match is ASCII case-insensitive. **Longest** needle in
//! [`COLORED_KEYWORD_TABLE`](crate::render::vocabulary_colors::COLORED_KEYWORD_TABLE)
//! wins; each entry maps to a [`color::keyword`](crate::render::theme::color::keyword) tint.
//! The proper noun **The House** is a two-word phrase (`The` + `House`) tinted
//! crimson with the score-pop polychrome shader ([`TextEffectId::Polychrome`]).
//! **The Moon** uses twilight polychrome ([`TextEffectId::MoonPolychrome`]).
//!
//! 3D cascade labels and streaming score popups reuse `LAPIS` / `RUBY` /
//! `RELIC_GOLD` / `PARCHMENT` for chips, mult, gold, and final totals — see
//! [`crate::render::score_popups`] and [`crate::scenes::gameplay::cascade_hud`].
//!
//! For paragraphs that use **safe inline markup** (`**`, `{{effect:…}}`, …)
//! together with the same per-word tinting, use [`crate::ui::widget::push_text_block`]
//! (`glossary_tint: true`) instead of this module’s plain-text wrappers.

use crate::render::decal::{load_mono_font, load_ui_font, load_ui_font_italic};
use crate::render::text_effect::TextEffectId;
use crate::render::theme::color;
#[allow(unused_imports)] // Re-exported for API parity; table lives in `vocabulary_colors`.
pub use crate::render::vocabulary_colors::COLORED_KEYWORD_TABLE;
#[allow(unused_imports)]
// Re-exported for API parity; implementation lives in `vocabulary_colors`.
pub use crate::render::vocabulary_colors::color_for_token;
pub use crate::render::vocabulary_colors::colored_token_segments;
use crate::render::wgpu_renderer::{TextAlign, TextLabel};
use crate::ui::clip::intersect_rect;
use crate::ui::text_wrap::{TextBreakUnit, break_units_kp};
use crate::ui::widget;

/// Vertical step between colored keyword rows ([`push_colored_rows_left`], tooltips, guide panels).
/// All measure helpers and push paths must use this — do not duplicate the multiplier elsewhere.
pub const COLORED_ROW_LINE_STEP_MUL: f32 = 1.4;

/// Dark stroke for glossary keyword tints on light panel fills (e.g. mint "Play" on white).
pub const KEYWORD_OUTLINE_COLOR: [f32; 4] = color::WALNUT_INK;
/// **The House** polychrome — crisp black rim so gold/crimson bands read on dark UI.
pub const HOUSE_OUTLINE_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// **The Moon** polychrome — twilight field rim so moonlight bands read on dark UI.
pub const MOON_OUTLINE_COLOR: [f32; 4] = color::keyword::MOON_OUTLINE;

#[inline]
pub fn keyword_is_tinted(segment_color: [f32; 4], default_color: [f32; 4]) -> bool {
    segment_color != default_color
}

#[inline]
fn keyword_outline_color(segment_color: [f32; 4]) -> [f32; 4] {
    if crate::render::vocabulary_colors::is_house_keyword_color(segment_color) {
        HOUSE_OUTLINE_COLOR
    } else if crate::render::vocabulary_colors::is_moon_keyword_color(segment_color) {
        MOON_OUTLINE_COLOR
    } else {
        KEYWORD_OUTLINE_COLOR
    }
}

#[inline]
fn proper_noun_polychrome_outline(segment_color: [f32; 4]) -> bool {
    crate::render::vocabulary_colors::is_house_keyword_color(segment_color)
        || crate::render::vocabulary_colors::is_moon_keyword_color(segment_color)
}

#[inline]
fn keyword_outline_offsets(font_px: f32, house: bool) -> [(f32, f32); 8] {
    let d = if house {
        (font_px * 0.068).clamp(1.5, 2.5)
    } else {
        (font_px * 0.055).clamp(1.0, 2.0)
    };
    [
        (-d, 0.0),
        (d, 0.0),
        (0.0, -d),
        (0.0, d),
        (-d, -d),
        (d, -d),
        (-d, d),
        (d, d),
    ]
}

/// Push `label`, optionally preceded by an ink outline when it is a glossary tint.
pub fn push_keyword_label(
    out: &mut Vec<TextLabel>,
    label: TextLabel,
    default_color: [f32; 4],
    outline_tinted: bool,
) {
    let font_px = label.font_px.unwrap_or(label.rect[3]);
    if outline_tinted && keyword_is_tinted(label.color, default_color) {
        let thick = proper_noun_polychrome_outline(label.color);
        for (dx, dy) in keyword_outline_offsets(font_px, thick) {
            let mut stroke = label.clone();
            stroke.rect[0] += dx;
            stroke.rect[1] += dy;
            stroke.color = keyword_outline_color(label.color);
            stroke.text_effect = TextEffectId::Flat;
            out.push(stroke);
        }
    }
    out.push(label);
}

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
    let wrapped = wrap_colored_words(text, inner_w, line_h, default, false);
    colored_wrapped_rows_height(&wrapped, line_h)
}

/// Height of [`wrap_colored_text_multiline`] output (focus inspect, `\n` in tooltips).
pub fn colored_multiline_text_height(
    text: &str,
    inner_w: f32,
    line_h: f32,
    default: [f32; 4],
) -> f32 {
    let lines = wrap_colored_text_multiline(text, inner_w, line_h, default, false);
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
            wrapped: wrap_colored_words(text, inner_w, line_h, default, false),
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
                italic: false,
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

/// Merge a punctuation-only chunk onto the previous chunk when both share a
/// color so raster kerning stays intact (see `colored_token_segments`).
fn glue_same_color_trailing_punct(line: &mut Vec<(String, [f32; 4])>) {
    let mut i = 1usize;
    while i < line.len() {
        if line[i].0 == " " {
            i += 1;
            continue;
        }
        if crate::render::vocabulary_colors::is_punctuation_only(&line[i].0)
            && line[i].1 == line[i - 1].1
            && line[i - 1].0 != " "
        {
            let (punct, _) = line.remove(i);
            line[i - 1].0.push_str(&punct);
            continue;
        }
        i += 1;
    }
}

fn word_width(font: &fontdue::Font, word: &str, font_px: f32) -> f32 {
    word.chars()
        .map(|ch| font.metrics(ch, font_px).advance_width)
        .sum()
}

fn wrap_measure_font(italic: bool) -> Option<&'static fontdue::Font> {
    if italic {
        load_ui_font_italic().or_else(load_ui_font)
    } else {
        load_ui_font()
    }
}

/// Extra width on the last segment of an italic row — glyph ink can extend past advance.
fn italic_trailing_slack(font_px: f32) -> f32 {
    font_px * 0.05
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
    italic: bool,
) -> Vec<Vec<(String, [f32; 4])>> {
    let Some(font) = wrap_measure_font(italic) else {
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

    let relic_mask = crate::render::vocabulary_colors::relic_name_word_mask(&words);
    let house_mask = crate::render::vocabulary_colors::house_name_word_mask(&words);

    let units: Vec<TextBreakUnit<Vec<(String, [f32; 4])>>> = words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let segments = if relic_mask[i] {
                vec![(w.to_string(), default)]
            } else if house_mask[i] {
                crate::render::vocabulary_colors::colored_token_segments_tinted(
                    w,
                    color::keyword::HOUSE,
                    default,
                )
            } else {
                crate::render::vocabulary_colors::colored_token_segments_with_next(
                    w,
                    words.get(i + 1).copied(),
                    default,
                )
            };
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
            glue_same_color_trailing_punct(&mut line);
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
    italic: bool,
) -> Vec<Vec<(String, [f32; 4])>> {
    if wrap_measure_font(italic).is_none() {
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
            italic,
        ));
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
    /// When true, measure and rasterize with the italic UI face (margin scrawl, etc.).
    pub italic: bool,
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
        italic,
    } = layout;
    let font_px = line_h;
    let line_step = colored_row_line_step(line_h);
    let Some(font) = wrap_measure_font(italic) else {
        let wrapped = widget::wrap_text(fallback_plain, inner_w, line_h);
        let joined = wrapped.join("\n");
        let h = colored_multiline_block_height(wrapped.len().max(1), line_h);
        out.push(TextLabel {
            rect: [text_left, top_y, inner_w, h],
            text: joined,
            color: fallback_color,
            font_px: Some(font_px),
            align: TextAlign::Left,
            italic,
            ..Default::default()
        });
        return;
    };

    for (row, chunks) in lines.iter().enumerate() {
        let line_y = top_y + row as f32 * line_step;
        let mut cx = text_left;
        push_tinted_segment_run(
            out,
            chunks,
            font,
            font_px,
            line_y,
            line_step,
            &mut cx,
            None,
            false,
            fallback_color,
            italic,
        );
    }
}

fn line_start_x(origin_x: f32, span_w: f32, total_w: f32, align: TextAlign) -> f32 {
    match align {
        TextAlign::Right => origin_x + span_w - total_w,
        TextAlign::Center => origin_x + (span_w - total_w) * 0.5,
        TextAlign::Left => origin_x,
    }
}

fn measure_tinted_run(font: &fontdue::Font, segments: &[(String, [f32; 4])], font_px: f32) -> f32 {
    segments
        .iter()
        .map(|(s, _)| word_width(font, s, font_px))
        .sum()
}

fn push_tinted_segment_run(
    out: &mut Vec<TextLabel>,
    segments: &[(String, [f32; 4])],
    font: &fontdue::Font,
    font_px: f32,
    y: f32,
    row_h: f32,
    cursor_x: &mut f32,
    clip_rect: Option<[f32; 4]>,
    mono: bool,
    default_color: [f32; 4],
    italic: bool,
) {
    let trailing_slack = if italic {
        italic_trailing_slack(font_px)
    } else {
        0.0
    };
    for (i, (s, c)) in segments.iter().enumerate() {
        let mut piece_w = word_width(font, s, font_px).max(1.0);
        if italic && i + 1 == segments.len() {
            piece_w += trailing_slack;
        }
        let text_effect =
            crate::render::vocabulary_colors::text_effect_for_glossary_tint(*c);
        push_keyword_label(
            out,
            TextLabel {
                rect: [*cursor_x, y, piece_w, row_h],
                text: s.clone(),
                color: *c,
                font_px: Some(font_px),
                align: TextAlign::Left,
                clip_rect,
                mono,
                italic,
                text_effect,
                ..Default::default()
            },
            default_color,
            true,
        );
        *cursor_x += piece_w;
    }
}

fn colored_line_segments(text: &str, default: [f32; 4]) -> Vec<(String, [f32; 4])> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let relic_mask = crate::render::vocabulary_colors::relic_name_word_mask(&words);
    let house_mask = crate::render::vocabulary_colors::house_name_word_mask(&words);
    let moon_mask = crate::render::vocabulary_colors::moon_name_word_mask(&words);
    let mut segments = Vec::new();
    for (wi, word) in words.iter().enumerate() {
        if wi > 0 {
            segments.push((" ".to_string(), default));
        }
        if relic_mask[wi] {
            segments.push(((*word).to_string(), default));
        } else if house_mask[wi] {
            segments.extend(crate::render::vocabulary_colors::colored_token_segments_tinted(
                word,
                color::keyword::HOUSE,
                default,
            ));
        } else if moon_mask[wi] {
            segments.extend(crate::render::vocabulary_colors::colored_token_segments_tinted(
                word,
                color::keyword::MOON,
                default,
            ));
        } else {
            segments.extend(crate::render::vocabulary_colors::colored_token_segments_with_next(
                word,
                words.get(wi + 1).copied(),
                default,
            ));
        }
    }
    if segments.is_empty() && !text.is_empty() {
        segments.push((text.to_string(), default));
    }
    glue_same_color_trailing_punct(&mut segments);
    segments
}

/// Single-line glossary tinting inside a clipped rect (Chronicle ledger cells).
pub fn push_colored_line_clipped(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    clip_rect: Option<[f32; 4]>,
    text: &str,
    default: [f32; 4],
    font_px: f32,
    align: TextAlign,
    mono: bool,
) {
    let clip = clip_rect.unwrap_or(rect);
    let Some(clipped) = intersect_rect(rect, clip) else {
        return;
    };
    let segments = colored_line_segments(text, default);
    let font = if mono {
        load_mono_font().or_else(load_ui_font)
    } else {
        load_ui_font()
    };
    let Some(font) = font else {
        out.push(TextLabel {
            rect: clipped,
            text: text.into(),
            color: default,
            font_px: Some(font_px),
            align,
            clip_rect: Some(clipped),
            mono,
            ..Default::default()
        });
        return;
    };
    let total_w = measure_tinted_run(font, &segments, font_px);
    let mut x = line_start_x(clipped[0], clipped[2], total_w, align);
    push_tinted_segment_run(
        out,
        &segments,
        font,
        font_px,
        clipped[1],
        clipped[3],
        &mut x,
        Some(clipped),
        mono,
        default,
        false,
    );
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
        italic,
    } = layout;
    let font_px = line_h;
    let line_step = colored_row_line_step(line_h);
    let Some(font) = wrap_measure_font(italic) else {
        let wrapped = widget::wrap_text(fallback_plain, inner_w, line_h);
        let joined = wrapped.join("\n");
        let h = colored_multiline_block_height(wrapped.len().max(1), line_h);
        out.push(TextLabel {
            rect: [block_left, top_y, inner_w, h],
            text: joined,
            color: fallback_color,
            font_px: Some(font_px),
            align,
            italic,
            ..Default::default()
        });
        return;
    };

    for (row, chunks) in lines.iter().enumerate() {
        let line_y = top_y + row as f32 * line_step;
        let total_w = measure_tinted_run(font, chunks, font_px);
        let mut cx = line_start_x(block_left, inner_w, total_w, align);
        push_tinted_segment_run(
            out,
            chunks,
            font,
            font_px,
            line_y,
            line_step,
            &mut cx,
            None,
            false,
            fallback_color,
            italic,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::theme::color;

    #[test]
    fn push_colored_line_left_matches_colored_line_block_height() {
        let text = "Manzu — ranks 1–9.";
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
    fn the_house_phrase_gets_crimson_polychrome() {
        let text = "Vital to beating The House.";
        let line_h = 22.0;
        let default = color::CHAMPAGNE;
        let mut labels = Vec::new();
        push_colored_line_left(&mut labels, 0.0, 0.0, 400.0, line_h, text, default);
        let house_tinted: Vec<_> = labels
            .iter()
            .filter(|l| l.color == color::keyword::HOUSE)
            .collect();
        let words: Vec<&str> = house_tinted.iter().map(|l| l.text.as_str()).collect();
        assert!(words.contains(&"The"));
        assert!(words.contains(&"House"));
        for l in house_tinted {
            assert_eq!(l.text_effect, TextEffectId::Polychrome);
        }
        let strokes: Vec<_> = labels
            .iter()
            .filter(|l| l.color == HOUSE_OUTLINE_COLOR)
            .collect();
        assert!(
            strokes.len() >= 8,
            "expected black outline strokes around House tokens"
        );
        for s in strokes {
            assert_eq!(s.text_effect, TextEffectId::Flat);
        }
    }

    #[test]
    fn melds_intro_punct_has_no_space_before_comma() {
        let text = "Melds are small tile groups — pairs, sequences, triplets, and kongs.";
        let line_h = 28.0 / 0.99;
        let default = color::PARCHMENT;
        for max_w in [200.0, 300.0, 400.0, 800.0] {
            let lines = wrap_colored_words(text, max_w, line_h, default, false);
            for (li, line) in lines.iter().enumerate() {
                let rendered: String = line.iter().map(|(s, _)| s.as_str()).collect();
                assert!(
                    !rendered.contains(" ,"),
                    "space before comma at width {max_w} line {li}: {rendered:?} chunks={line:?}"
                );
                assert!(
                    !rendered.contains(" ."),
                    "space before period at width {max_w} line {li}: {rendered:?}"
                );
            }
        }
        let text2 = "Valid melds can be played into your structure.";
        for max_w in [200.0, 300.0, 400.0, 800.0] {
            let lines = wrap_colored_words(text2, max_w, line_h, default, false);
            for line in &lines {
                let rendered: String = line.iter().map(|(s, _)| s.as_str()).collect();
                assert!(
                    !rendered.contains(" ."),
                    "space before period at width {max_w}: {rendered:?} chunks={line:?}"
                );
            }
        }
    }

    #[test]
    fn colored_wrap_keeps_trailing_punct_with_word() {
        let font_px = 28.0;
        let line_h = font_px / 0.99;
        let text = "An East Wind is blowing!";
        let font = load_ui_font().expect("ui font");
        let max_w = word_width(font, "An East Wind is blowing", font_px) + 2.0;
        let default = [1.0, 1.0, 1.0, 1.0];
        let lines = wrap_colored_words(text, max_w, line_h, default, false);
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
