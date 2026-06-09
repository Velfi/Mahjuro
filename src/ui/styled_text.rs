//! Unified player-facing UI text: safe inline markup + glossary vocabulary tints.
//!
//! # Grammar (whitelist)
//!
//! - **Bold**: `**` toggles bold until the next `**` (toggle semantics).
//! - *Italic*: `*` toggles italic (must not be part of `**` — `**` is checked first).
//! - __Underline__: `__` toggles underline.
//! - **Effects**: `{{effect:name}}` … `{{/effect}}` — curated names only; see
//!   [`crate::render::text_effect::TextEffectId::from_markup_name`].
//! - **Glossary terms**: `{{term:Honors}}` — force a vocabulary tint (preserves tag casing).
//! - **Escapes**: `\` before `*`, `_`, `{`, `}`, `\` emits the literal next character.
//!
//! # Glossary modes
//!
//! [`GlossaryMode`] controls auto-tinting via [`crate::render::vocabulary_colors`]:
//! - `Off` — plain text
//! - `Prose` — mixed English + jargon (suppresses ambiguous title-cased honor words)
//! - `Panel` — guide glossary rows (always tint table hits)
//!
//! # Limits
//!
//! [`MAX_STYLED_INPUT_BYTES`], [`MAX_EFFECT_STACK`], [`MAX_STYLED_RUNS`].

use crate::render::decal::{load_mono_font, load_ui_font, load_ui_font_italic};
use crate::render::text_effect::TextEffectId;
use crate::render::theme::{color, typography};
pub use crate::render::vocabulary_colors::COLORED_KEYWORD_TABLE;
use crate::render::vocabulary_colors::{
    GlossaryMode, colored_token_segments, glossary_word_segments, glossary_word_segments_forced,
    text_effect_for_glossary_tint,
};
use crate::render::wgpu_renderer::{TextAlign, TextLabel};
use crate::ui::clip::intersect_rect;
use crate::ui::text_wrap::{TextBreakUnit, break_units_kp};
use crate::ui::widget::{self, TextStyle};

/// Vertical step between glossary text rows. All measure/push paths use this multiplier.
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

#[inline]
pub fn ui_text_line_step(line_h: f32) -> f32 {
    colored_row_line_step(line_h)
}

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
/// `{{term:` — opening delimiter before a forced glossary term.
const TAG_TERM_OPEN_PREFIX_LEN: usize = 7;

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
    /// From `{{term:…}}` — bypass title-case guard in [`GlossaryMode::Prose`].
    pub force_glossary: bool,
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
            && last.force_glossary == r.force_glossary
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
    force_glossary: bool,
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
        force_glossary,
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
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff, false)?;
            let _ = effect_stack.pop();
            i += TAG_CLOSE_EFFECT_LEN;
            continue;
        }

        // {{effect:name}}
        if char_slice_starts_with_str(&chars, i, "{{effect:") {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff, false)?;
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

        // {{term:name}}
        if char_slice_starts_with_str(&chars, i, "{{term:") {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff, false)?;
            let start = i + TAG_TERM_OPEN_PREFIX_LEN;
            let mut j = start;
            while j + 1 < chars.len() && !(chars[j] == '}' && chars[j + 1] == '}') {
                j += 1;
            }
            if j + 1 >= chars.len() {
                return Err(StyledParseError::UnclosedEffectRegion);
            }
            let term: String = chars[start..j].iter().collect();
            if runs.len() >= MAX_STYLED_RUNS {
                return Err(StyledParseError::TooManyRuns);
            }
            runs.push(StyledRun {
                text: term,
                bold,
                italic,
                underline,
                effect: eff,
                force_glossary: true,
            });
            i = j + 2;
            continue;
        }

        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff, false)?;
            bold = !bold;
            i += 2;
            continue;
        }

        if i + 1 < chars.len() && chars[i] == '_' && chars[i + 1] == '_' {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff, false)?;
            underline = !underline;
            i += 2;
            continue;
        }

        if chars[i] == '*' {
            let eff = active_effect(&effect_stack);
            flush_styled_run(&mut buf, &mut runs, bold, italic, underline, eff, false)?;
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
        false,
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
                    force_glossary: false,
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
                force_glossary: false,
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

fn push_glossary_word_cells(
    cells: &mut Vec<Cell>,
    word: &str,
    words: &[&str],
    word_idx: usize,
    run: &StyledRun,
    mode: GlossaryMode,
    default_color: [f32; 4],
) {
    if word.is_empty() {
        return;
    }
    let segments = if run.force_glossary {
        glossary_word_segments_forced(words, word_idx, mode, default_color, true)
    } else {
        glossary_word_segments(words, word_idx, mode, default_color)
    };
    for (segment, col) in segments {
        let effect = match text_effect_for_glossary_tint(col) {
            TextEffectId::Flat => run.effect,
            fx => fx,
        };
        for ch in segment.chars() {
            cells.push(Cell {
                ch,
                bold: run.bold,
                italic: run.italic,
                underline: run.underline,
                effect,
                color: col,
            });
        }
    }
}

fn runs_to_cells_with_glossary(
    runs: &[StyledRun],
    default_color: [f32; 4],
    mode: GlossaryMode,
) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    if matches!(mode, GlossaryMode::Off) {
        for run in runs {
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
        }
        return cells;
    }

    let joined: String = runs.iter().map(|r| r.text.as_str()).collect();
    let all_words: Vec<&str> = joined.split_whitespace().collect();
    let mut word_idx = 0usize;

    let flush_word = |cells: &mut Vec<Cell>,
                      word: &mut String,
                      run: &StyledRun,
                      word_idx: &mut usize| {
        if word.is_empty() {
            return;
        }
        if *word_idx < all_words.len() {
            push_glossary_word_cells(cells, word, &all_words, *word_idx, run, mode, default_color);
            *word_idx += 1;
        } else {
            for ch in word.chars() {
                cells.push(Cell {
                    ch,
                    bold: run.bold,
                    italic: run.italic,
                    underline: run.underline,
                    effect: run.effect,
                    color: default_color,
                });
            }
        }
        word.clear();
    };

    for run in runs {
        let mut word = String::new();
        for ch in run.text.chars() {
            if ch == '\n' {
                flush_word(&mut cells, &mut word, run, &mut word_idx);
                cells.push(Cell {
                    ch: '\n',
                    bold: run.bold,
                    italic: run.italic,
                    underline: run.underline,
                    effect: run.effect,
                    color: default_color,
                });
                continue;
            }
            if ch.is_whitespace() {
                flush_word(&mut cells, &mut word, run, &mut word_idx);
                cells.push(Cell {
                    ch,
                    bold: run.bold,
                    italic: run.italic,
                    underline: run.underline,
                    effect: run.effect,
                    color: default_color,
                });
            } else {
                word.push(ch);
            }
        }
        flush_word(&mut cells, &mut word, run, &mut word_idx);
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
    bold: bool,
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
    let mut adv = face.metrics(ch, font_px).advance_width;
    if bold {
        adv += crate::render::decal::FAUX_BOLD_OVERLAY_OFFSET_PX;
    }
    adv
}

fn cell_token_advance(
    tok: &[Cell],
    font_px: f32,
    regular: Option<&fontdue::Font>,
    italic_f: Option<&fontdue::Font>,
) -> f32 {
    tok.iter()
        .map(|c| char_advance_styled(c.ch, c.bold, c.italic, font_px, regular, italic_f))
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
        let ch_w = char_advance_styled(c.ch, c.bold, c.italic, font_px, regular, italic_f);
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

fn layout_styled_visual_lines_at_font_px(
    text: &str,
    max_width_px: f32,
    font_px: f32,
    glossary: GlossaryMode,
    default_color: [f32; 4],
) -> Vec<Vec<Cell>> {
    let runs = parse_styled_text_lossy(text);
    let cells = runs_to_cells_with_glossary(&runs, default_color, glossary);
    let hard_lines = split_lines_by_newline(&cells);
    let regular = load_ui_font();
    let italic = load_ui_font_italic();
    let italic_f = italic.or(regular);
    let mut visual_lines: Vec<Vec<Cell>> = Vec::new();
    for hl in hard_lines {
        if hl.is_empty() {
            visual_lines.push(Vec::new());
            continue;
        }
        let mut wrapped = wrap_cells_hard(&hl, max_width_px, font_px, regular, italic_f);
        visual_lines.append(&mut wrapped);
    }
    if visual_lines.is_empty() {
        visual_lines.push(Vec::new());
    }
    visual_lines
}

/// Pre-measured styled copy — use [`Self::measure_at_font_px`] then [`Self::line_count`],
/// [`Self::block_height`], and [`Self::push_at_font_px`] so layout cannot drift.
pub struct StyledTextBlock {
    visual_lines: Vec<Vec<Cell>>,
    font_px: f32,
}

impl StyledTextBlock {
    pub fn measure_at_font_px(
        text: &str,
        max_width_px: f32,
        font_px: f32,
        glossary: GlossaryMode,
        default_color: [f32; 4],
    ) -> Self {
        Self {
            visual_lines: layout_styled_visual_lines_at_font_px(
                text,
                max_width_px,
                font_px,
                glossary,
                default_color,
            ),
            font_px,
        }
    }

    pub fn measure(
        text: &str,
        max_width_px: f32,
        tier: f32,
        window_h: f32,
        glossary: GlossaryMode,
        default_color: [f32; 4],
    ) -> Self {
        Self::measure_at_font_px(
            text,
            max_width_px,
            typography::size(tier, window_h),
            glossary,
            default_color,
        )
    }

    /// Line count after markup parse + wrap (same rules as [`push_styled_text_block`]).
    pub fn line_count(&self) -> usize {
        self.visual_lines.len().max(1)
    }

    /// Block height after markup parse + wrap (uses [`colored_row_line_step`]).
    pub fn block_height(&self) -> f32 {
        colored_row_line_step(self.font_px) * self.line_count() as f32
    }

    pub fn push_at_font_px(
        &self,
        out: &mut Vec<TextLabel>,
        rect: [f32; 4],
        style: StyledBlockStyle,
    ) {
        push_styled_visual_lines(out, rect, &self.visual_lines, self.font_px, style);
    }
}

/// Block height after markup parse + wrap (uses [`colored_row_line_step`]).
pub fn styled_line_block_height_at_font_px(
    text: &str,
    max_width_px: f32,
    font_px: f32,
    glossary: GlossaryMode,
    default_color: [f32; 4],
) -> f32 {
    StyledTextBlock::measure_at_font_px(text, max_width_px, font_px, glossary, default_color)
        .block_height()
}

pub fn styled_line_block_height(
    text: &str,
    max_width_px: f32,
    tier: f32,
    window_h: f32,
    glossary: GlossaryMode,
    default_color: [f32; 4],
) -> f32 {
    StyledTextBlock::measure(text, max_width_px, tier, window_h, glossary, default_color)
        .block_height()
}

fn push_styled_visual_lines(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    visual_lines: &[Vec<Cell>],
    font_px: f32,
    style: StyledBlockStyle,
) {
    let [x, y, w, h] = rect;
    let pad = style.padding;
    let inner_w = (w - 2.0 * pad).max(1.0);
    let inner_h = (h - 2.0 * pad).max(1.0);
    let line_step = colored_row_line_step(font_px);
    let max_lines = ((inner_h / line_step).floor() as usize).max(1);

    let regular = load_ui_font();
    let italic = load_ui_font_italic();
    let italic_f = italic.or(regular);

    let drawn: Vec<_> = visual_lines.iter().take(max_lines).collect();
    let n = drawn.len().max(1);
    let block_h = inner_h.min(n as f32 * line_step);
    let base_y = match style.vertical_align {
        Some(crate::render::wgpu_renderer::TextBlockVerticalAlign::Top) => y + pad,
        Some(crate::render::wgpu_renderer::TextBlockVerticalAlign::Bottom) => {
            y + pad + inner_h - block_h
        }
        None => y + pad + (inner_h - block_h) * 0.5,
    };

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
            push_keyword_label(
                out,
                TextLabel {
                    rect: [cx, line_y, piece_w, line_step],
                    text: chunk.text,
                    color: chunk.color,
                    font_px: Some(font_px),
                    align: TextAlign::Left,
                    underline: chunk.underline,
                    text_effect: chunk.effect,
                    bold: chunk.bold,
                    italic: chunk.italic,
                    flavor_spans: None,
                    ..Default::default()
                },
                style.color,
                style.glossary.tints(),
            );
            cx += piece_w;
        }
    }
}

/// Push styled copy at an explicit font size (tutorial column scaling uses this).
pub fn push_styled_text_block_at_font_px(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
    style: StyledBlockStyle,
) {
    let [_, _, w, _] = rect;
    let inner_w = (w - 2.0 * style.padding).max(1.0);
    StyledTextBlock::measure_at_font_px(text, inner_w, font_px, style.glossary, style.color)
        .push_at_font_px(out, rect, style);
}

/// Alias for [`StyledTextBlock`] — unified UI text measure/push.
pub type UiTextBlock = StyledTextBlock;

/// Style for [`push_styled_text_block`].
#[derive(Clone, Copy, Debug)]
pub struct StyledBlockStyle {
    pub tier: f32,
    pub color: [f32; 4],
    pub padding: f32,
    pub align: TextAlign,
    /// Per-word vocabulary tint mode inside styled runs.
    pub glossary: GlossaryMode,
    /// When set, pins the block to the top or bottom of `rect`; `None` keeps legacy centering.
    pub vertical_align: Option<crate::render::wgpu_renderer::TextBlockVerticalAlign>,
}

impl Default for StyledBlockStyle {
    fn default() -> Self {
        Self {
            tier: typography::H36,
            color: crate::render::theme::color::PARCHMENT,
            padding: 0.0,
            align: TextAlign::Center,
            glossary: GlossaryMode::Off,
            vertical_align: None,
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
            glossary: s.glossary,
            vertical_align: None,
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
    push_styled_text_block_at_font_px(
        out,
        rect,
        text,
        typography::size(style.tier, window_h),
        style,
    );
}

// --- Plain-text glossary layout (former `colored_keywords` API) ---

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

fn italic_trailing_slack(font_px: f32) -> f32 {
    font_px * 0.05
}

pub fn colored_wrapped_rows_height(rows: &[Vec<(String, [f32; 4])>], line_h: f32) -> f32 {
    colored_row_line_step(line_h) * rows.len().max(1) as f32
}

pub fn colored_line_block_height(
    text: &str,
    inner_w: f32,
    line_h: f32,
    default: [f32; 4],
    glossary: GlossaryMode,
) -> f32 {
    let wrapped = wrap_colored_words(text, inner_w, line_h, default, false, glossary);
    colored_wrapped_rows_height(&wrapped, line_h)
}

pub fn colored_multiline_text_height(
    text: &str,
    inner_w: f32,
    line_h: f32,
    default: [f32; 4],
    glossary: GlossaryMode,
) -> f32 {
    let lines = wrap_colored_text_multiline(text, inner_w, line_h, default, false, glossary);
    colored_wrapped_rows_height(&lines, line_h)
}

pub fn colored_lines_block_height(
    lines: &[&str],
    inner_w: f32,
    line_h: f32,
    default: [f32; 4],
    glossary: GlossaryMode,
) -> f32 {
    lines
        .iter()
        .map(|line| colored_line_block_height(line, inner_w, line_h, default, glossary))
        .sum()
}

pub struct ColoredLineBlock {
    wrapped: Vec<Vec<(String, [f32; 4])>>,
    line_h: f32,
}

impl ColoredLineBlock {
    pub fn measure(
        text: &str,
        inner_w: f32,
        line_h: f32,
        default: [f32; 4],
        glossary: GlossaryMode,
    ) -> Self {
        Self {
            wrapped: wrap_colored_words(text, inner_w, line_h, default, false, glossary),
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
        glossary: GlossaryMode,
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
                glossary,
            },
            &self.wrapped,
        );
    }
}

pub fn push_colored_line_left(
    out: &mut Vec<TextLabel>,
    text_left: f32,
    top_y: f32,
    inner_w: f32,
    line_h: f32,
    text: &str,
    default: [f32; 4],
    glossary: GlossaryMode,
) -> f32 {
    let block = ColoredLineBlock::measure(text, inner_w, line_h, default, glossary);
    let h = block.height();
    block.push_left(out, text_left, top_y, inner_w, text, default, glossary);
    h
}

pub fn colored_paragraph_preferred_width(
    text: &str,
    line_h: f32,
    max_width_px: f32,
    glossary: GlossaryMode,
) -> f32 {
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
            for (seg, _) in colored_token_segments(word, default, glossary) {
                line_w += word_width(font, &seg, font_px);
            }
        }
        widest = widest.max(line_w);
    }
    widest.clamp(0.0, max_width_px)
}

pub fn ui_text_preferred_width(
    text: &str,
    line_h: f32,
    max_width_px: f32,
    glossary: GlossaryMode,
) -> f32 {
    colored_paragraph_preferred_width(text, line_h, max_width_px, glossary)
}

pub fn wrap_colored_words(
    text: &str,
    max_width_px: f32,
    line_h: f32,
    default: [f32; 4],
    italic: bool,
    glossary: GlossaryMode,
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

    let units: Vec<TextBreakUnit<Vec<(String, [f32; 4])>>> = words
        .iter()
        .enumerate()
        .map(|(i, _w)| {
            let segments = glossary_word_segments(&words, i, glossary, default);
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

pub fn wrap_colored_text_multiline(
    text: &str,
    max_width_px: f32,
    line_h: f32,
    default: [f32; 4],
    italic: bool,
    glossary: GlossaryMode,
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
            glossary,
        ));
    }
    if out.is_empty() {
        out.push(vec![(String::new(), default)]);
    }
    out
}

pub fn colored_multiline_block_height(line_count: usize, line_h: f32) -> f32 {
    colored_row_line_step(line_h) * line_count as f32
}

pub struct ColoredRowsLayout<'a> {
    pub text_left: f32,
    pub top_y: f32,
    pub inner_w: f32,
    pub line_h: f32,
    pub fallback_plain: &'a str,
    pub fallback_color: [f32; 4],
    pub italic: bool,
    pub glossary: GlossaryMode,
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

/// Largest font size ≤ `target_px` so tinted segments fit `max_w`.
fn fit_tinted_line_font_px(
    font: &fontdue::Font,
    segments: &[(String, [f32; 4])],
    max_w: f32,
    target_px: f32,
) -> f32 {
    let min_px = 8.0f32;
    let target_px = target_px.max(min_px);
    if max_w <= 0.0 || measure_tinted_run(font, segments, target_px) <= max_w {
        return target_px;
    }
    let mut lo = min_px;
    let mut hi = target_px;
    for _ in 0..12 {
        let mid = (lo + hi) * 0.5;
        if measure_tinted_run(font, segments, mid) <= max_w {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
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
    glossary: GlossaryMode,
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
        let text_effect = text_effect_for_glossary_tint(*c);
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
            glossary.tints(),
        );
        *cursor_x += piece_w;
    }
}

fn colored_line_segments(
    text: &str,
    default: [f32; 4],
    glossary: GlossaryMode,
) -> Vec<(String, [f32; 4])> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut segments = Vec::new();
    for (wi, _word) in words.iter().enumerate() {
        if wi > 0 {
            segments.push((" ".to_string(), default));
        }
        segments.extend(glossary_word_segments(&words, wi, glossary, default));
    }
    if segments.is_empty() && !text.is_empty() {
        segments.push((text.to_string(), default));
    }
    glue_same_color_trailing_punct(&mut segments);
    segments
}

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
        glossary,
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
            glossary,
        );
    }
}

pub fn push_colored_line_clipped(
    out: &mut Vec<TextLabel>,
    rect: [f32; 4],
    clip_rect: Option<[f32; 4]>,
    text: &str,
    default: [f32; 4],
    font_px: f32,
    align: TextAlign,
    mono: bool,
    glossary: GlossaryMode,
) {
    let clip = clip_rect.unwrap_or(rect);
    let Some(clipped) = intersect_rect(rect, clip) else {
        return;
    };
    let segments = colored_line_segments(text, default, glossary);
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
    let fit_px = fit_tinted_line_font_px(font, &segments, clipped[2], font_px);
    let total_w = measure_tinted_run(font, &segments, fit_px);
    let mut x = line_start_x(clipped[0], clipped[2], total_w, align);
    push_tinted_segment_run(
        out,
        &segments,
        font,
        fit_px,
        clipped[1],
        clipped[3],
        &mut x,
        Some(clipped),
        mono,
        default,
        false,
        glossary,
    );
}

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
        glossary,
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
            glossary,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_house_glossary_tint_uses_score_pop_polychrome() {
        let runs = parse_styled_text("Vital to beating The House.").unwrap();
        let cells = runs_to_cells_with_glossary(
            &runs,
            crate::render::theme::color::CHAMPAGNE,
            GlossaryMode::Prose,
        );
        let house_cells: Vec<_> = cells
            .iter()
            .filter(|c| c.color == crate::render::theme::color::keyword::HOUSE)
            .collect();
        assert!(!house_cells.is_empty());
        for c in house_cells {
            assert_eq!(c.effect, TextEffectId::Polychrome);
        }
    }

    #[test]
    fn glossary_tint_preserves_whitespace_around_bold() {
        let runs = parse_styled_text("Select tiles. **Discard** what you don't need.").unwrap();
        let cells = runs_to_cells_with_glossary(
            &runs,
            crate::render::theme::color::PARCHMENT,
            GlossaryMode::Prose,
        );
        let text: String = cells.iter().map(|c| c.ch).collect();
        assert_eq!(text, "Select tiles. Discard what you don't need.");
    }

    #[test]
    fn styled_measure_matches_layout_line_count() {
        let text = "Tap **Play**, then **Cash In**. Try **Discard**.";
        let font_px = 18.0;
        let w = 240.0;
        let color = crate::render::theme::color::PARCHMENT;
        let block =
            StyledTextBlock::measure_at_font_px(text, w, font_px, GlossaryMode::Prose, color);
        let visual =
            layout_styled_visual_lines_at_font_px(text, w, font_px, GlossaryMode::Prose, color);
        assert_eq!(block.line_count(), visual.len().max(1));
        assert_eq!(
            styled_line_block_height_at_font_px(text, w, font_px, GlossaryMode::Prose, color),
            block.block_height(),
        );
    }

    #[test]
    fn term_markup_forces_tint_in_prose() {
        let runs = parse_styled_text("{{term:Honors}} cannot form sequences.").unwrap();
        let d = [0.5, 0.5, 0.5, 1.0];
        let cells = runs_to_cells_with_glossary(&runs, d, GlossaryMode::Prose);
        let honors: Vec<_> = cells
            .iter()
            .filter(|c| c.color == crate::render::theme::color::keyword::HONORS)
            .collect();
        assert!(!honors.is_empty());
    }

    #[test]
    fn panel_mode_tints_title_cased_honors() {
        let d = [0.5, 0.5, 0.5, 1.0];
        let lines = wrap_colored_words(
            "Honors cannot form sequences.",
            400.0,
            22.0,
            d,
            false,
            GlossaryMode::Panel,
        );
        let rendered: String = lines[0].iter().map(|(s, _)| s.as_str()).collect();
        assert!(rendered.contains("Honors"));
        assert!(
            lines[0]
                .iter()
                .any(|(_, c)| *c == crate::render::theme::color::keyword::HONORS)
        );
    }

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
