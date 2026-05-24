//! Safe inline markup for player-facing UI copy (not full Markdown).
//!
//! # Grammar (whitelist)
//!
//! - **Bold**: `**` toggles bold until the next `**` (toggle semantics).
//! - *Italic*: `*` toggles italic (must not be part of `**` — `**` is checked first).
//! - __Underline__: `__` toggles underline.
//! - **Effects**: `{{effect:name}}` … `{{/effect}}` — curated names only; see
//!   [`crate::render::text_effect::TextEffectId::from_markup_name`].
//! - **Escapes**: `\` before `*`, `_`, `{`, `}`, `\` emits the literal next character.
//!
//! # Limits
//!
//! [`MAX_STYLED_INPUT_BYTES`], [`MAX_EFFECT_STACK`], [`MAX_STYLED_RUNS`].
//!
//! # Glossary tint vs colored keywords
//!
//! [`StyledBlockStyle::glossary_tint`] (and [`crate::ui::widget::TextStyle::glossary_tint`]) turns on
//! per-word colors using the same [`crate::render::vocabulary_colors::color_for_token`] table
//! documented in [`crate::ui::colored_keywords`]. That module also provides word-wrap helpers for
//! plain copy; styled markup goes through this module instead.

use crate::render::decal::{load_ui_font, load_ui_font_italic};
use crate::render::text_effect::TextEffectId;
use crate::render::theme::typography;
use crate::render::vocabulary_colors::colored_token_segments;
use crate::render::wgpu_renderer::{TextAlign, TextLabel};
use crate::ui::widget::TextStyle;

/// Max UTF-8 bytes accepted by the parser.
pub const MAX_STYLED_INPUT_BYTES: usize = 12_000;
/// Max nested `{{effect:…}}` depth.
pub const MAX_EFFECT_STACK: usize = 16;
/// Max styled runs emitted (after merge).
pub const MAX_STYLED_RUNS: usize = 128;

/// `{{/effect}}` — ASCII, same length in bytes and `char`s.
const TAG_CLOSE_EFFECT_LEN: usize = 11;
/// `{{effect:` — opening delimiter before the effect name.
const TAG_EFFECT_OPEN_PREFIX_LEN: usize = 9;

#[inline]
fn active_effect(effect_stack: &[TextEffectId]) -> TextEffectId {
    effect_stack.last().copied().unwrap_or(TextEffectId::Flat)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StyledParseError {
    InputTooLong { len: usize },
    UnclosedEffectRegion,
    EffectStackOverflow,
    TooManyRuns,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyledRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub effect: TextEffectId,
}

fn merge_adjacent_runs(runs: Vec<StyledRun>) -> Vec<StyledRun> {
    if runs.is_empty() {
        return runs;
    }
    let mut out: Vec<StyledRun> = Vec::with_capacity(runs.len());
    for r in runs {
        if r.text.is_empty() {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.bold == r.bold
            && last.italic == r.italic
            && last.underline == r.underline
            && last.effect == r.effect
        {
            last.text.push_str(&r.text);
        } else {
            out.push(r);
        }
    }
    out
}

fn flush_styled_run(
    buf: &mut String,
    runs: &mut Vec<StyledRun>,
    bold: bool,
    italic: bool,
    underline: bool,
    effect: TextEffectId,
) -> Result<(), StyledParseError> {
    if buf.is_empty() {
        return Ok(());
    }
    if runs.len() >= MAX_STYLED_RUNS {
        return Err(StyledParseError::TooManyRuns);
    }
    runs.push(StyledRun {
        text: std::mem::take(buf),
        bold,
        italic,
        underline,
        effect,
    });
    Ok(())
}

/// Strict parse for tests; UI uses [`parse_styled_text_lossy`].
pub fn parse_styled_text(input: &str) -> Result<Vec<StyledRun>, StyledParseError> {
    if input.len() > MAX_STYLED_INPUT_BYTES {
        return Err(StyledParseError::InputTooLong { len: input.len() });
    }
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    let mut buf = String::new();
    let mut runs: Vec<StyledRun> = Vec::new();

    let mut bold = false;
    let mut italic = false;
    let mut underline = false;
    let mut effect_stack: Vec<TextEffectId> = Vec::new();

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // {{/effect}}
        if char_slice_starts_with_str(&chars, i, "{{/effect}}") {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff)?;
            let _ = effect_stack.pop();
            i += TAG_CLOSE_EFFECT_LEN;
            continue;
        }

        // {{effect:name}}
        if char_slice_starts_with_str(&chars, i, "{{effect:") {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff)?;
            let start = i + TAG_EFFECT_OPEN_PREFIX_LEN;
            let mut j = start;
            while j + 1 < chars.len() && !(chars[j] == '}' && chars[j + 1] == '}') {
                j += 1;
            }
            if j + 1 >= chars.len() {
                return Err(StyledParseError::UnclosedEffectRegion);
            }
            let name: String = chars[start..j].iter().collect();
            let id = TextEffectId::from_markup_name(&name).unwrap_or(TextEffectId::Flat);
            if effect_stack.len() >= MAX_EFFECT_STACK {
                return Err(StyledParseError::EffectStackOverflow);
            }
            effect_stack.push(id);
            i = j + 2;
            continue;
        }

        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff)?;
            bold = !bold;
            i += 2;
            continue;
        }

        if i + 1 < chars.len() && chars[i] == '_' && chars[i + 1] == '_' {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff)?;
            underline = !underline;
            i += 2;
            continue;
        }

        if chars[i] == '*' {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff)?;
            italic = !italic;
            i += 1;
            continue;
        }

        buf.push(chars[i]);
        i += 1;
    }

    flush_styled_run(
        &mut buf,
        &mut runs,
        bold,
        italic,
        underline,
        active_effect(&effect_stack),
    )?;

    if !effect_stack.is_empty() {
        return Err(StyledParseError::UnclosedEffectRegion);
    }

    Ok(merge_adjacent_runs(runs))
}

/// Prefix must be ASCII-only so its UTF-8 length matches `char` count.
fn char_slice_starts_with_str(chars: &[char], i: usize, prefix: &str) -> bool {
    let mut j = i;
    for pc in prefix.chars() {
        if chars.get(j).copied() != Some(pc) {
            return false;
        }
        j += 1;
    }
    true
}

/// Fall back to a single plain run on parse errors.
pub fn parse_styled_text_lossy(input: &str) -> Vec<StyledRun> {
    match parse_styled_text(input) {
        Ok(r) => {
            if r.is_empty() {
                vec![StyledRun {
                    text: input.to_string(),
                    bold: false,
                    italic: false,
                    underline: false,
                    effect: TextEffectId::Flat,
                }]
            } else {
                r
            }
        }
        Err(e) => {
            log::debug!("styled_text parse: {e:?}, using plain");
            vec![StyledRun {
                text: input.to_string(),
                bold: false,
                italic: false,
                underline: false,
                effect: TextEffectId::Flat,
            }]
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    ch: char,
    bold: bool,
    italic: bool,
    underline: bool,
    effect: TextEffectId,
    color: [f32; 4],
}

fn runs_to_cells_with_glossary(
    runs: &[StyledRun],
    default_color: [f32; 4],
    glossary: bool,
) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    for run in runs {
        if !glossary {
            for ch in run.text.chars() {
                cells.push(Cell {
                    ch,
                    bold: run.bold,
                    italic: run.italic,
                    underline: run.underline,
                    effect: run.effect,
                    color: default_color,
                });
            }
            continue;
        }
        for (pi, para) in run.text.split('\n').enumerate() {
            if pi > 0 {
                cells.push(Cell {
                    ch: '\n',
                    bold: run.bold,
                    italic: run.italic,
                    underline: run.underline,
                    effect: run.effect,
                    color: default_color,
                });
            }
            let mut first_word = true;
            for word in para.split_whitespace() {
                if !first_word {
                    cells.push(Cell {
                        ch: ' ',
                        bold: run.bold,
                        italic: run.italic,
                        underline: run.underline,
                        effect: run.effect,
                        color: default_color,
                    });
                }
                first_word = false;
                for (segment, col) in colored_token_segments(word, default_color) {
                    for ch in segment.chars() {
                        cells.push(Cell {
                            ch,
                            bold: run.bold,
                            italic: run.italic,
                            underline: run.underline,
                            effect: run.effect,
                            color: col,
                        });
                    }
                }
            }
        }
    }
    cells
}

fn tokenize_cells(cells: &[Cell]) -> Vec<&[Cell]> {
    if cells.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<&[Cell]> = Vec::new();
    let mut i = 0;
    while i < cells.len() {
        let ws = cells[i].ch.is_whitespace();
        let mut j = i + 1;
        while j < cells.len() && cells[j].ch.is_whitespace() == ws {
            j += 1;
        }
        out.push(&cells[i..j]);
        i = j;
    }
    out
}

fn char_advance_styled(
    ch: char,
    italic: bool,
    font_px: f32,
    regular: Option<&fontdue::Font>,
    italic_f: Option<&fontdue::Font>,
) -> f32 {
    let Some(regular) = regular else {
        return font_px * 0.5;
    };
    let face = if italic {
        italic_f.filter(|f| f.has_glyph(ch)).unwrap_or(regular)
    } else {
        regular
    };
    face.metrics(ch, font_px).advance_width
}

fn cell_token_advance(
    tok: &[Cell],
    font_px: f32,
    regular: Option<&fontdue::Font>,
    italic_f: Option<&fontdue::Font>,
) -> f32 {
    tok.iter()
        .map(|c| char_advance_styled(c.ch, c.italic, font_px, regular, italic_f))
        .sum()
}

fn trim_trailing_ws(line: &mut Vec<Cell>) {
    while line.last().is_some_and(|c| c.ch.is_whitespace()) {
        line.pop();
    }
}

fn wrap_cells_hard(
    cells: &[Cell],
    max_w: f32,
    font_px: f32,
    regular: Option<&fontdue::Font>,
    italic_f: Option<&fontdue::Font>,
) -> Vec<Vec<Cell>> {
    let space_w = regular
        .map(|f| f.metrics(' ', font_px).advance_width)
        .unwrap_or(font_px * 0.25);

    let tokens = tokenize_cells(cells);
    if tokens.is_empty() {
        return vec![Vec::new()];
    }
    let mut lines: Vec<Vec<Cell>> = Vec::new();
    let mut line: Vec<Cell> = Vec::new();
    let mut line_w = 0.0_f32;

    for tok in &tokens {
        let is_ws = tok[0].ch.is_whitespace();
        let tw = cell_token_advance(tok, font_px, regular, italic_f);
        if is_ws {
            if line.is_empty() {
                continue;
            }
            if line_w + tw <= max_w {
                line.extend_from_slice(tok);
                line_w += tw;
            }
            continue;
        }
        if line.is_empty() {
            line.extend_from_slice(tok);
            line_w = tw;
            continue;
        }
        let gap = if line.last().is_some_and(|c| c.ch.is_whitespace()) {
            0.0
        } else {
            space_w
        };
        if line_w + gap + tw <= max_w {
            if gap > 0.0 {
                let sample = tok[0];
                line.push(Cell {
                    ch: ' ',
                    bold: sample.bold,
                    italic: sample.italic,
                    underline: sample.underline,
                    effect: sample.effect,
                    color: sample.color,
                });
                line_w += space_w;
            }
            line.extend_from_slice(tok);
            line_w += tw;
        } else {
            trim_trailing_ws(&mut line);
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            line.extend_from_slice(tok);
            line_w = tw;
        }
    }
    trim_trailing_ws(&mut line);
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn split_lines_by_newline(cells: &[Cell]) -> Vec<Vec<Cell>> {
    let mut hard: Vec<Vec<Cell>> = vec![Vec::new()];
    for c in cells {
        if c.ch == '\n' {
            hard.push(Vec::new());
        } else {
            hard.last_mut().unwrap().push(*c);
        }
    }
    hard
}

/// One merged substring on a wrapped line (same face + color + shader effect).
#[derive(Clone, Debug, PartialEq)]
pub struct LineTextChunk {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub effect: TextEffectId,
    pub color: [f32; 4],
    /// Width sum matching [`crate::render::decal::rasterize_label_raster_spans`] layout (metrics advance only).
    pub advance_width: f32,
}

fn merge_cells_for_runs(
    line: &[Cell],
    font_px: f32,
    regular: Option<&fontdue::Font>,
    italic_f: Option<&fontdue::Font>,
) -> Vec<LineTextChunk> {
    if line.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<LineTextChunk> = Vec::new();
    for c in line {
        let ch_w = char_advance_styled(c.ch, c.italic, font_px, regular, italic_f);
        if let Some(last) = out.last_mut()
            && last.bold == c.bold
            && last.italic == c.italic
            && last.underline == c.underline
            && last.effect == c.effect
            && last.color == c.color
        {
            last.text.push(c.ch);
            last.advance_width += ch_w;
        } else {
            out.push(LineTextChunk {
                text: c.ch.to_string(),
                bold: c.bold,
                italic: c.italic,
                underline: c.underline,
                effect: c.effect,
                color: c.color,
                advance_width: ch_w,
            });
        }
    }
    out
}

/// Style for [`push_styled_text_block`].
#[derive(Clone, Copy, Debug)]
pub struct StyledBlockStyle {
    pub tier: f32,
    pub color: [f32; 4],
    pub padding: f32,
    pub align: TextAlign,
    /// When true, apply [`color_for_token`] per word inside styled runs.
    pub glossary_tint: bool,
}

impl Default for StyledBlockStyle {
    fn default() -> Self {
        Self {
            tier: typography::H36,
            color: crate::render::theme::color::PARCHMENT,
            padding: 0.0,
            align: TextAlign::Center,
            glossary_tint: false,
        }
    }
}

impl From<TextStyle> for StyledBlockStyle {
    fn from(s: TextStyle) -> Self {
        Self {
            tier: s.tier,
            color: s.color,
            padding: s.padding,
            align: s.align,
            glossary_tint: s.glossary_tint,
        }
    }
}

/// Parse markup, wrap, and push one [`TextLabel`] per merged chunk per line
/// (same raster style + color + effect).
pub fn push_styled_text_block(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    text: &str,
    style: StyledBlockStyle,
    window_h: f32,
) {
    let runs = parse_styled_text_lossy(text);
    let [x, y, w, h] = rect;
    let pad = style.padding;
    let inner_w = (w - 2.0 * pad).max(1.0);
    let inner_h = (h - 2.0 * pad).max(1.0);
    let line_h = typography::size(style.tier, window_h);
    let font_px = line_h;
    let line_step = crate::ui::colored_keywords::colored_row_line_step(line_h);
    let max_lines = ((inner_h / line_step).floor() as usize).max(1);

    let regular = load_ui_font();
    let italic = load_ui_font_italic();
    let italic_f = italic.or(regular);

    let cells = runs_to_cells_with_glossary(&runs, style.color, style.glossary_tint);
    let hard_lines = split_lines_by_newline(&cells);
    let mut visual_lines: Vec<Vec<Cell>> = Vec::new();
    for hl in hard_lines {
        if hl.is_empty() {
            visual_lines.push(Vec::new());
            continue;
        }
        let mut wrapped = wrap_cells_hard(&hl, inner_w, font_px, regular, italic_f);
        visual_lines.append(&mut wrapped);
    }
    if visual_lines.is_empty() {
        visual_lines.push(Vec::new());
    }
    let drawn: Vec<_> = visual_lines.into_iter().take(max_lines).collect();
    let n = drawn.len().max(1);
    let block_h = inner_h.min(n as f32 * line_step);
    let base_y = y + pad + (inner_h - block_h) * 0.5;

    for (row, line_cells) in drawn.iter().enumerate() {
        if line_cells.is_empty() {
            continue;
        }
        let chunks = merge_cells_for_runs(line_cells, font_px, regular, italic_f);
        let line_y = base_y + row as f32 * line_step;
        let total_w: f32 = chunks.iter().map(|c| c.advance_width).sum();
        let mut cx = match style.align {
            TextAlign::Left => x + pad,
            TextAlign::Center => x + pad + (inner_w - total_w) * 0.5,
            TextAlign::Right => x + pad + inner_w - total_w,
        };
        for chunk in chunks {
            let piece_w = chunk.advance_width.max(1.0);
            out.push(TextLabel {
                rect: [cx, line_y, piece_w, line_step],
                text: chunk.text,
                color: chunk.color,
                font_px: Some(font_px),
                align: TextAlign::Left,
                no_glossary: true,
                underline: chunk.underline,
                text_effect: chunk.effect,
                bold: chunk.bold,
                italic: chunk.italic,
                flavor_spans: None,
                ..Default::default()
            });
            cx += piece_w;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bold_toggle() {
        let r = parse_styled_text("a**b**c").unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].text, "a");
        assert!(!r[0].bold);
        assert_eq!(r[1].text, "b");
        assert!(r[1].bold);
        assert_eq!(r[2].text, "c");
        assert!(!r[2].bold);
    }

    #[test]
    fn parse_effect_region() {
        let r = parse_styled_text("{{effect:rainbow}}x{{/effect}}").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "x");
        assert_eq!(r[0].effect, TextEffectId::Rainbow);
    }

    #[test]
    fn escape_stars() {
        let r = parse_styled_text(r"a\*b").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "a*b");
    }

    #[test]
    fn lossy_on_unclosed_effect() {
        let r = parse_styled_text_lossy("{{effect:rainbow}}x");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "{{effect:rainbow}}x");
    }

    #[test]
    fn nested_effect_inner_wins() {
        let r =
            parse_styled_text("{{effect:rainbow}}{{effect:pulse}}x{{/effect}}{{/effect}}").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "x");
        assert_eq!(r[0].effect, TextEffectId::Pulse);
    }

    #[test]
    fn escape_brace() {
        let r = parse_styled_text(r"a\{b").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "a{b");
    }

    #[test]
    fn parse_errors_when_too_many_runs() {
        let mut s = String::new();
        for i in 0..129 {
            let eff = if i % 2 == 0 { "rainbow" } else { "pulse" };
            use std::fmt::Write;
            write!(&mut s, "{{{{effect:{eff}}}}}z{{{{/effect}}}}").unwrap();
        }
        assert_eq!(parse_styled_text(&s), Err(StyledParseError::TooManyRuns));
    }

    #[test]
    fn lossy_on_too_many_runs() {
        let mut s = String::new();
        for i in 0..129 {
            let eff = if i % 2 == 0 { "rainbow" } else { "pulse" };
            use std::fmt::Write;
            write!(&mut s, "{{{{effect:{eff}}}}}z{{{{/effect}}}}").unwrap();
        }
        let r = parse_styled_text_lossy(&s);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].effect, TextEffectId::Flat);
    }

    #[test]
    fn markup_inside_region_keeps_face_flags() {
        let r = parse_styled_text("{{effect:gold}}**x**{{/effect}}").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "x");
        assert!(r[0].bold);
        assert_eq!(r[0].effect, TextEffectId::GoldTint);
    }
}
