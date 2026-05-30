//! Reusable Kenney controller / keyboard hint rows.
//!
//! Two layouts:
//! - **Inline** — compact centred `[icon]: verb · …` copy (archive inspect, profile delete).
//! - **Column** — equal-width `[icon][label]` slots with optional pill backers (shop, gameplay).

use crate::render::decal::{load_ui_font, measure_label_advances};
use crate::render::draw_cmd::{ImageQuad, ImageQuadSource, UiFrame};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::scenes::DrawCtx;
use crate::scenes::object3d_inspect::ITEM_INSPECT_OVERLAY_EXIT;
use crate::ui::glyph_source::{GlyphResolver, StickSide, TriggerSide};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::kenney_prompt_paths::keyboard_key;

const INLINE_SEP: &str = "   ·   ";
const INLINE_SLASH: &str = " / ";
const INLINE_PLUS: &str = "+";

const HINT_ICON_PX_MIN: f32 = 48.0;
const HINT_ICON_PX_MAX: f32 = 132.0;
const HINT_BAR_H_FRAC: f32 = 0.056;
const HINT_ICON_BAR_FRAC: f32 = 0.72;

// ── Shared sizing (shop legend is the reference) ─────────────────────────────

/// Icon + label metrics shared by inline and column control hints.
#[derive(Clone, Copy, Debug)]
pub struct HintMetrics {
    pub icon_px: f32,
    pub legend_font_px: f32,
    pub legend_line_h: f32,
    pub row_height: f32,
    pub gap_after_icon: f32,
}

impl HintMetrics {
    pub fn primary(h: f32) -> Self {
        let bar_h_ref = h * HINT_BAR_H_FRAC;
        let icon_px =
            (bar_h_ref * HINT_ICON_BAR_FRAC * 3.0).clamp(HINT_ICON_PX_MIN, HINT_ICON_PX_MAX);
        let legend_font_px = typography::size(typography::H20, h);
        let ui_font = load_ui_font();
        let legend_line_h = ui_font
            .as_ref()
            .and_then(|f| f.horizontal_line_metrics(legend_font_px))
            .map(|lm| lm.new_line_size)
            .unwrap_or(legend_font_px * 1.2)
            .max(legend_font_px * 0.85);
        let caption_px = typography::size(typography::H42, h);
        let row_height = (icon_px * 1.06).max(legend_line_h).max(caption_px * 1.35);
        let gap_after_icon = icon_px * 0.18;
        Self {
            icon_px,
            legend_font_px,
            legend_line_h,
            row_height,
            gap_after_icon,
        }
    }
}

// ── Shared key resolution ───────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptInputSurface {
    Controller,
    MouseOrKeyboard,
}

/// How multiple icons in one bind are joined visually.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HintKeyJoin {
    #[default]
    Slash,
    Plus,
    Tight,
}

/// One resolved icon slot (controller action, stick, trigger, or keyboard atlas name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKey {
    Action(UiAction),
    Stick(StickSide),
    Trigger(TriggerSide),
    Keyboard(&'static str),
}

impl HintKey {
    pub fn for_input(input_mode: InputMode, action: UiAction, keyboard: &'static str) -> Self {
        match input_mode {
            InputMode::Controller => Self::Action(action),
            InputMode::Keyboard | InputMode::Cursor => Self::Keyboard(keyboard),
        }
    }
}

fn prompt_surface(input_mode: InputMode) -> PromptInputSurface {
    match input_mode {
        InputMode::Controller => PromptInputSurface::Controller,
        InputMode::Keyboard | InputMode::Cursor => PromptInputSurface::MouseOrKeyboard,
    }
}

fn resolve_hint_key(
    glyphs: GlyphResolver,
    surface: PromptInputSurface,
    key: HintKey,
) -> Option<ImageQuadSource> {
    match (surface, key) {
        (PromptInputSurface::Controller, HintKey::Action(action)) => glyphs.glyph_for(action),
        (PromptInputSurface::Controller, HintKey::Stick(side)) => glyphs.stick_glyph(side),
        (PromptInputSurface::Controller, HintKey::Trigger(side)) => glyphs.trigger_glyph(side),
        (PromptInputSurface::MouseOrKeyboard, HintKey::Keyboard(name)) => Some(keyboard_key(name)),
        _ => None,
    }
}

// ── Inline layout ───────────────────────────────────────────────────────────

/// One labelled bind: slash between groups, `within_join` inside each group.
#[derive(Clone, Debug)]
pub struct HintBind {
    pub label: String,
    pub key_groups: Vec<Vec<HintKey>>,
    pub within_join: HintKeyJoin,
}

impl HintBind {
    pub fn alternatives(label: impl Into<String>, keys: Vec<HintKey>) -> Self {
        Self {
            label: label.into(),
            key_groups: vec![keys],
            within_join: HintKeyJoin::Slash,
        }
    }

    pub fn grouped(
        label: impl Into<String>,
        key_groups: Vec<Vec<HintKey>>,
        within_join: HintKeyJoin,
    ) -> Self {
        Self {
            label: label.into(),
            key_groups,
            within_join,
        }
    }
}

impl From<HintBind> for HintSegment {
    fn from(bind: HintBind) -> Self {
        Self::Bind(bind)
    }
}

/// One chunk in a centred inline hint row.
#[derive(Clone, Debug)]
pub enum HintSegment {
    Sep,
    PlainText(String),
    Bind(HintBind),
}

impl HintSegment {
    pub fn sep() -> Self {
        Self::Sep
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::PlainText(text.into())
    }

    pub fn bind(label: impl Into<String>, keys: Vec<HintKey>) -> Self {
        HintBind::alternatives(label, keys).into()
    }

    pub fn bind_join(label: impl Into<String>, keys: Vec<HintKey>, within_join: HintKeyJoin) -> Self {
        HintBind::grouped(label, vec![keys], within_join).into()
    }

    /// Slash separates groups; `within_join` joins keys inside each group.
    pub fn bind_groups(
        label: impl Into<String>,
        key_groups: Vec<Vec<HintKey>>,
        within_join: HintKeyJoin,
    ) -> Self {
        HintBind::grouped(label, key_groups, within_join).into()
    }
}

/// Builder for [`HintSegment`] rows.
#[derive(Clone, Debug, Default)]
pub struct HintRow {
    segments: Vec<HintSegment>,
}

impl HintRow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(mut self, segment: HintSegment) -> Self {
        self.segments.push(segment);
        self
    }

    pub fn bind(self, label: impl Into<String>, keys: Vec<HintKey>) -> Self {
        self.push(HintSegment::bind(label, keys))
    }

    pub fn bind_join(
        self,
        label: impl Into<String>,
        keys: Vec<HintKey>,
        within_join: HintKeyJoin,
    ) -> Self {
        self.push(HintSegment::bind_join(label, keys, within_join))
    }

    pub fn bind_groups(
        self,
        label: impl Into<String>,
        key_groups: Vec<Vec<HintKey>>,
        within_join: HintKeyJoin,
    ) -> Self {
        self.push(HintSegment::bind_groups(label, key_groups, within_join))
    }

    pub fn sep(mut self) -> Self {
        self.segments.push(HintSegment::sep());
        self
    }

    pub fn into_segments(self) -> Vec<HintSegment> {
        self.segments
    }
}

/// Visual parameters for inline hint rows.
#[derive(Clone, Copy, Debug)]
pub struct HintStyle {
    pub font_px: f32,
    pub line_h: f32,
    pub icon_px: f32,
    pub gap_after_icon: f32,
    pub text_color: [f32; 4],
    pub icon_tint: [f32; 4],
}

impl HintStyle {
    fn from_metrics(metrics: HintMetrics, text_color: [f32; 4], icon_tint: [f32; 4]) -> Self {
        Self {
            font_px: metrics.legend_font_px,
            line_h: metrics.row_height,
            icon_px: metrics.icon_px,
            gap_after_icon: metrics.gap_after_icon,
            text_color,
            icon_tint,
        }
    }

    pub fn archive_footer(h: f32) -> Self {
        Self::from_metrics(
            HintMetrics::primary(h),
            [0.78, 0.80, 0.88, 0.92],
            color::alpha(color::PORCELAIN_AGED, 0.94),
        )
    }

    pub fn profile_footer(h: f32) -> Self {
        Self::from_metrics(
            HintMetrics::primary(h),
            color::UMBER,
            color::alpha(color::UMBER, 0.96),
        )
    }

    pub fn shop_inspect_camera(h: f32) -> Self {
        Self::from_metrics(
            HintMetrics::primary(h),
            [0.82, 0.78, 0.72, 0.94],
            color::alpha(color::PORCELAIN_AGED, 0.94),
        )
    }
}

/// Kenney keyboard sprite for `action` when building inline inspect hints.
fn inspect_overlay_exit_hint_key(input_mode: InputMode, action: UiAction) -> Option<HintKey> {
    match (input_mode, action) {
        (InputMode::Controller, UiAction::Cancel | UiAction::NorthFacePress) => {
            Some(HintKey::Action(action))
        }
        (InputMode::Controller, UiAction::Pause) => None,
        (InputMode::Keyboard | InputMode::Cursor, UiAction::Cancel) => {
            Some(HintKey::Keyboard("keyboard_backspace"))
        }
        (InputMode::Keyboard | InputMode::Cursor, UiAction::Pause) => {
            Some(HintKey::Keyboard("keyboard_escape"))
        }
        (InputMode::Keyboard | InputMode::Cursor, UiAction::NorthFacePress) => {
            Some(HintKey::Keyboard("keyboard_e"))
        }
        _ => None,
    }
}

fn inspect_orbit_bind(input_mode: InputMode) -> HintBind {
    if matches!(input_mode, InputMode::Controller) {
        HintBind::grouped(
            "orbit",
            vec![vec![HintKey::Stick(StickSide::Right)]],
            HintKeyJoin::Tight,
        )
    } else {
        HintBind::alternatives(
            "orbit",
            vec![
                HintKey::Keyboard("keyboard_arrows"),
                HintKey::Keyboard("mouse_move"),
            ],
        )
    }
}

fn inspect_zoom_bind(input_mode: InputMode) -> HintBind {
    if matches!(input_mode, InputMode::Controller) {
        HintBind::alternatives(
            "zoom",
            vec![
                HintKey::Trigger(TriggerSide::Left),
                HintKey::Trigger(TriggerSide::Right),
            ],
        )
    } else {
        HintBind::grouped(
            "zoom",
            vec![
                vec![
                    HintKey::Keyboard("keyboard_shift"),
                    HintKey::Keyboard("keyboard_arrows_vertical"),
                ],
                vec![HintKey::Keyboard("mouse_scroll_vertical")],
            ],
            HintKeyJoin::Plus,
        )
    }
}

fn inspect_exit_bind(input_mode: InputMode) -> HintBind {
    let keys: Vec<HintKey> = ITEM_INSPECT_OVERLAY_EXIT
        .iter()
        .filter_map(|&action| inspect_overlay_exit_hint_key(input_mode, action))
        .collect();
    HintBind::alternatives("exit", keys)
}

/// Camera + exit controls while item inspect is active (item cycling uses normal focus nav).
pub fn inspect_camera_hint_row(input_mode: InputMode) -> Vec<HintSegment> {
    HintRow::new()
        .push(inspect_orbit_bind(input_mode).into())
        .sep()
        .push(inspect_zoom_bind(input_mode).into())
        .sep()
        .push(inspect_exit_bind(input_mode).into())
        .into_segments()
}

enum InlineSegmentRef<'a> {
    Sep,
    PlainText(&'a str),
    Bind(&'a HintBind),
}

enum InlineBindPart {
    GroupSep,
    WithinSep,
    Key(HintKey, bool),
}

fn for_each_inline_bind_part(
    bind: &HintBind,
    mut f: impl FnMut(InlineBindPart),
) {
    let within_text = key_join_text(bind.within_join);
    for (gi, group) in bind.key_groups.iter().enumerate() {
        if gi > 0 {
            f(InlineBindPart::GroupSep);
        }
        for (ki, &key) in group.iter().enumerate() {
            if ki > 0 && !within_text.is_empty() {
                f(InlineBindPart::WithinSep);
            }
            let last_icon =
                gi + 1 == bind.key_groups.len() && ki + 1 == group.len();
            f(InlineBindPart::Key(key, last_icon));
        }
    }
}

fn key_join_text(join: HintKeyJoin) -> &'static str {
    match join {
        HintKeyJoin::Slash => INLINE_SLASH,
        HintKeyJoin::Plus => INLINE_PLUS,
        HintKeyJoin::Tight => "",
    }
}

fn inline_segment_refs<'a>(
    scratch: &'a mut Vec<InlineSegmentRef<'a>>,
    owned: &'a [HintSegment],
) -> &'a [InlineSegmentRef<'a>] {
    scratch.clear();
    scratch.reserve(owned.len());
    for seg in owned {
        scratch.push(match seg {
            HintSegment::Sep => InlineSegmentRef::Sep,
            HintSegment::PlainText(text) => InlineSegmentRef::PlainText(text),
            HintSegment::Bind(bind) => InlineSegmentRef::Bind(bind),
        });
    }
    scratch.as_slice()
}

fn measure_text(font_px: f32, text: &str) -> f32 {
    let text_h_px = font_px.max(8.0).round().max(1.0) as u32;
    if let Some(font) = load_ui_font() {
        let (_, _, advances) = measure_label_advances(font, text, 8192, text_h_px, Some(font_px));
        advances.iter().copied().sum()
    } else {
        let est_ch = text.chars().count().max(1) as f32;
        (font_px * 0.52 * est_ch).max(8.0)
    }
}

fn bind_suffix(label: &str) -> String {
    format!(": {label}")
}

fn measure_inline_bind(
    glyphs: GlyphResolver,
    surface: PromptInputSurface,
    icon_px: f32,
    gap_after_icon: f32,
    font_px: f32,
    bind: &HintBind,
) -> f32 {
    let within_text = key_join_text(bind.within_join);
    let mut w = 0.0;
    for_each_inline_bind_part(bind, |part| match part {
        InlineBindPart::GroupSep => w += measure_text(font_px, INLINE_SLASH),
        InlineBindPart::WithinSep => w += measure_text(font_px, within_text),
        InlineBindPart::Key(key, last_icon) => {
            if resolve_hint_key(glyphs, surface, key).is_some() {
                w += icon_px;
                if last_icon {
                    w += gap_after_icon;
                }
            }
        }
    });
    w + measure_text(font_px, &bind_suffix(&bind.label))
}

fn measure_inline_row(
    glyphs: GlyphResolver,
    surface: PromptInputSurface,
    icon_px: f32,
    gap_after_icon: f32,
    font_px: f32,
    segments: &[InlineSegmentRef<'_>],
) -> f32 {
    segments.iter().fold(0.0, |acc, seg| {
        acc + match *seg {
            InlineSegmentRef::Sep => measure_text(font_px, INLINE_SEP),
            InlineSegmentRef::PlainText(text) => measure_text(font_px, text),
            InlineSegmentRef::Bind(bind) => {
                measure_inline_bind(glyphs, surface, icon_px, gap_after_icon, font_px, bind)
            }
        }
    })
}

fn emit_inline_row(
    glyphs: GlyphResolver,
    surface: PromptInputSurface,
    icon_px: f32,
    gap_after_icon: f32,
    style: HintStyle,
    rect: [f32; 4],
    segments: &[InlineSegmentRef<'_>],
    icon_cmds: &mut Vec<ImageQuad>,
    texts: &mut Vec<TextLabel>,
) {
    let [rx, ry, rw, rh] = rect;
    let row_w = measure_inline_row(
        glyphs,
        surface,
        icon_px,
        gap_after_icon,
        style.font_px,
        segments,
    );
    let mut x = rx + (rw - row_w).max(0.0) * 0.5;
    let iy = ry + (rh - icon_px).max(0.0) * 0.5;
    let text_y = ry + (rh - style.line_h).max(0.0) * 0.5;

    for seg in segments {
        match *seg {
            InlineSegmentRef::Sep => {
                let w = measure_text(style.font_px, INLINE_SEP);
                texts.push(inline_text_label(
                    [x, text_y, w, style.line_h],
                    INLINE_SEP,
                    style,
                ));
                x += w;
            }
            InlineSegmentRef::PlainText(text) => {
                let w = measure_text(style.font_px, text);
                texts.push(inline_text_label(
                    [x, text_y, w.max(1.0), style.line_h],
                    text,
                    style,
                ));
                x += w.max(1.0);
            }
            InlineSegmentRef::Bind(bind) => {
                let within_text = key_join_text(bind.within_join);
                for_each_inline_bind_part(bind, |part| match part {
                    InlineBindPart::GroupSep => {
                        let w = measure_text(style.font_px, INLINE_SLASH);
                        texts.push(inline_text_label(
                            [x, text_y, w, style.line_h],
                            INLINE_SLASH,
                            style,
                        ));
                        x += w;
                    }
                    InlineBindPart::WithinSep => {
                        let w = measure_text(style.font_px, within_text);
                        texts.push(inline_text_label(
                            [x, text_y, w, style.line_h],
                            within_text,
                            style,
                        ));
                        x += w;
                    }
                    InlineBindPart::Key(key, last_icon) => {
                        if let Some(source) = resolve_hint_key(glyphs, surface, key) {
                            icon_cmds.push(ImageQuad {
                                inst: GpuInstance {
                                    rect: [x, iy, icon_px, icon_px],
                                    color: style.icon_tint,
                                    user: 0,
                                },
                                source,
                            });
                            x += icon_px;
                            if last_icon {
                                x += gap_after_icon;
                            }
                        }
                    }
                });
                let suffix = bind_suffix(&bind.label);
                let w = measure_text(style.font_px, &suffix);
                texts.push(inline_text_label(
                    [x, text_y, w.max(1.0), style.line_h],
                    &suffix,
                    style,
                ));
                x += w.max(1.0);
            }
        }
    }
}

/// Push one or more centred inline hint rows.
pub fn push_inline_hint_rows(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    row_rects: &[[f32; 4]],
    rows: &[Vec<HintSegment>],
    style: HintStyle,
) {
    if row_rects.is_empty() || rows.is_empty() || row_rects.len() != rows.len() {
        return;
    }

    let surface = prompt_surface(ctx.input_mode);
    let glyphs = ctx.glyphs;
    let mut icon_px = style.icon_px;
    let mut gap_after_icon = style.gap_after_icon;

    let max_row_w = row_rects
        .iter()
        .map(|r| r[2])
        .fold(0.0_f32, f32::max)
        .max(1.0);
    loop {
        let fits = rows.iter().all(|row| {
            let mut scratch: Vec<InlineSegmentRef<'_>> = Vec::new();
            let segs = inline_segment_refs(&mut scratch, row);
            measure_inline_row(
                glyphs,
                surface,
                icon_px,
                gap_after_icon,
                style.font_px,
                segs,
            ) <= max_row_w
        });
        if fits || icon_px <= 18.0 {
            break;
        }
        icon_px -= 1.0;
        gap_after_icon = icon_px * 0.18;
    }

    let mut icon_cmds: Vec<ImageQuad> = Vec::new();
    let mut texts: Vec<TextLabel> = Vec::new();

    for (row, rect) in rows.iter().zip(row_rects.iter()) {
        let mut scratch: Vec<InlineSegmentRef<'_>> = Vec::new();
        let segs = inline_segment_refs(&mut scratch, row);
        emit_inline_row(
            glyphs,
            surface,
            icon_px,
            gap_after_icon,
            style,
            *rect,
            segs,
            &mut icon_cmds,
            &mut texts,
        );
    }

    if !icon_cmds.is_empty() {
        frame.image_quads(icon_cmds);
    }
    if !texts.is_empty() {
        frame.texts(texts);
    }
}

fn inline_text_label(rect: [f32; 4], text: &str, style: HintStyle) -> TextLabel {
    TextLabel {
        rect,
        text: text.to_string(),
        color: style.text_color,
        font_px: Some(style.font_px),
        align: TextAlign::Left,
        ..Default::default()
    }
}

// ── Column layout ─────────────────────────────────────────────────────────────

/// One equal-width column in a floating hint band.
#[derive(Clone, Debug)]
pub struct ColumnHintEntry {
    pub controller: HintKey,
    pub keyboard: ImageQuadSource,
    /// When set, layout is `label_before` + icon + `label` (e.g. "Hold " + Q + " Sell").
    pub label_before: Option<&'static str>,
    pub label: &'static str,
    pub disabled: bool,
}

impl ColumnHintEntry {
    pub fn new(controller: HintKey, keyboard: ImageQuadSource, label: &'static str) -> Self {
        Self {
            controller,
            keyboard,
            label_before: None,
            label,
            disabled: false,
        }
    }

    /// `before` + bind icon + `after` (shop hold-to-sell, etc.).
    pub fn surrounding_icon(
        controller: HintKey,
        keyboard: ImageQuadSource,
        before: &'static str,
        after: &'static str,
    ) -> Self {
        Self {
            controller,
            keyboard,
            label_before: Some(before),
            label: after,
            disabled: false,
        }
    }
}

/// Placement for a column hint band.
#[derive(Clone, Copy, Debug)]
pub struct ColumnHintLayout {
    pub inner_left: f32,
    pub inner_width: f32,
    pub row_top: f32,
    pub row_height: f32,
    pub column_count: usize,
    /// When set, used instead of the default floating-band icon cap.
    pub icon_px: Option<f32>,
}

impl ColumnHintLayout {
    /// Shop-style bottom band: 90% window width, inset 2%.
    pub fn shop_floating_band(w: f32, h: f32, inspect_active: bool, column_count: usize) -> Self {
        let pad_bottom = h * 0.014;
        let x = w * 0.05;
        let bw = w * 0.90;
        let inner_left = x + bw * 0.02;
        let inner_right = x + bw * 0.98;
        let inner_width = (inner_right - inner_left).max(8.0);

        let metrics = HintMetrics::primary(h);
        let inspect_line_h = if inspect_active {
            metrics.row_height
        } else {
            0.0
        };
        let gap_before_inspect_line = if inspect_active { h * 0.01 } else { 0.0 };
        let block_h = metrics.row_height + gap_before_inspect_line + inspect_line_h;
        let row_top = h - pad_bottom - block_h;

        Self {
            inner_left,
            inner_width,
            row_top,
            row_height: metrics.row_height,
            column_count: column_count.max(1),
            icon_px: Some(metrics.icon_px),
        }
    }

    /// Gameplay-style bottom band (non-inspect icon scale).
    pub fn gameplay_floating_band(w: f32, h: f32, column_count: usize) -> Self {
        Self::shop_floating_band(w, h, false, column_count)
    }
}

/// Visual parameters for column hint rows.
#[derive(Clone, Copy, Debug)]
pub struct ColumnHintStyle {
    pub legend_font_px: f32,
    pub icon_scale: f32,
    pub icon_tint: [f32; 4],
    pub label_color: [f32; 4],
    pub disabled_icon_tint: [f32; 4],
    pub disabled_label_color: [f32; 4],
    pub pill_bg: Option<[f32; 4]>,
    pub pill_bg_disabled: Option<[f32; 4]>,
    pub label_pad_x: f32,
    pub label_pad_y: f32,
}

impl ColumnHintStyle {
    pub fn shop_floating(h: f32) -> Self {
        let metrics = HintMetrics::primary(h);
        let pill_bg = [0.06_f32, 0.055, 0.07, 0.82];
        Self {
            legend_font_px: metrics.legend_font_px,
            icon_scale: 1.0,
            icon_tint: color::alpha(color::PORCELAIN_AGED, 0.96),
            label_color: color::alpha(color::PORCELAIN_AGED, 0.96),
            disabled_icon_tint: color::alpha(color::PORCELAIN_AGED, 0.96),
            disabled_label_color: color::alpha(color::PORCELAIN_AGED, 0.96),
            pill_bg: Some(pill_bg),
            pill_bg_disabled: Some(pill_bg),
            label_pad_x: 0.0,
            label_pad_y: 0.0,
        }
    }

    pub fn gameplay_floating(h: f32) -> Self {
        let metrics = HintMetrics::primary(h);
        let pill_bg = [0.06_f32, 0.055, 0.07, 0.82];
        Self {
            legend_font_px: metrics.legend_font_px,
            icon_scale: 1.0,
            icon_tint: color::alpha(color::PORCELAIN_AGED, 0.96),
            label_color: color::alpha(color::PORCELAIN_AGED, 0.96),
            disabled_icon_tint: color::alpha(
                color::darken(color::alpha(color::PORCELAIN_AGED, 0.96), 0.45),
                0.5,
            ),
            disabled_label_color: color::alpha(color::UMBER, 0.72),
            pill_bg: Some(pill_bg),
            pill_bg_disabled: Some([0.045_f32, 0.042, 0.048, 0.55]),
            label_pad_x: 4.0,
            label_pad_y: 3.0,
        }
    }
}

/// Per-column geometry after [`push_column_hints`].
#[derive(Clone, Copy, Debug)]
pub struct ColumnHintSlot {
    pub column_index: usize,
    pub icon_rect: [f32; 4],
}

/// Render a column hint row; returns slot geometry (icon rects) for overlays such as hold rings.
pub fn push_column_hints(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    layout: ColumnHintLayout,
    entries: &[ColumnHintEntry],
    style: ColumnHintStyle,
    text_out: &mut Vec<TextLabel>,
) -> Vec<ColumnHintSlot> {
    if entries.is_empty() {
        return Vec::new();
    }

    let h = ctx.layout.window_h;
    let surface = prompt_surface(ctx.input_mode);
    let glyphs = ctx.glyphs;
    let metrics = HintMetrics::primary(h);

    let col_w = layout.inner_width / layout.column_count as f32;
    let col_pad = (col_w * 0.045).clamp(2.0, 8.0);

    let mut icon_px = layout.icon_px.unwrap_or(metrics.icon_px) * style.icon_scale;
    let mut gap_after_icon = icon_px * 0.18;

    let ui_font = load_ui_font();
    let legend_line_h = metrics.legend_line_h;
    let legend_text_h_px = legend_line_h.max(8.0).round().max(1.0) as u32;
    let label_block_h = legend_line_h + style.label_pad_y * 2.0;

    let measure_label = |text: &str| -> f32 {
        if let Some(font) = ui_font {
            let (_, _, advances) = measure_label_advances(
                font,
                text,
                8192,
                legend_text_h_px,
                Some(style.legend_font_px),
            );
            advances.iter().copied().sum()
        } else {
            let est_ch = text.chars().count().max(1) as f32;
            (style.legend_font_px * 0.52 * est_ch).max(8.0)
        }
    };

    let measured: Vec<(f32, f32)> = entries
        .iter()
        .map(|entry| {
            let before_w = entry
                .label_before
                .map(measure_label)
                .unwrap_or(0.0);
            let after_w = measure_label(entry.label);
            (before_w, after_w)
        })
        .collect();

    loop {
        let mut fits = true;
        for (i, entry) in entries.iter().enumerate().take(entries.len()) {
            let (before_w, after_w) = measured[i];
            let label_inner = (before_w + after_w) + style.label_pad_x * 2.0;
            let cluster = match entry.label_before {
                None => icon_px + gap_after_icon + label_inner,
                Some(_) => {
                    before_w + gap_after_icon + icon_px + gap_after_icon + label_inner
                }
            };
            if cluster > col_w - col_pad * 2.0 {
                fits = false;
                break;
            }
        }
        if fits || icon_px <= 18.0 {
            break;
        }
        icon_px -= 1.0;
        gap_after_icon = icon_px * 0.18;
    }

    let row_h = layout.row_height;
    let iy = layout.row_top + (row_h - icon_px) * 0.5;
    let label_top = layout.row_top + (row_h - label_block_h) * 0.5;
    let pill_pad_x = (icon_px * 0.10).clamp(6.0, 16.0) + (h * 0.003).clamp(4.0, 8.0);
    let pill_pad_y = if style.label_pad_y > 0.0 {
        style.label_pad_y
    } else {
        (legend_line_h * 0.14).clamp(3.0, 9.0)
    };

    let mut pill_quads: Vec<GpuInstance> = Vec::with_capacity(entries.len());
    let mut icon_cmds: Vec<ImageQuad> = Vec::with_capacity(entries.len());
    let mut slots: Vec<ColumnHintSlot> = Vec::with_capacity(entries.len());

    for (col_i, entry) in entries.iter().enumerate().take(layout.column_count) {
        let col_x = layout.inner_left + col_i as f32 * col_w;
        let cluster_left = col_x + col_pad;
        let (before_w, after_w) = measured[col_i];
        let max_cluster_w = (col_w - col_pad * 2.0).max(10.0);

        let icon_tint = if entry.disabled {
            style.disabled_icon_tint
        } else {
            style.icon_tint
        };
        let label_color = if entry.disabled {
            style.disabled_label_color
        } else {
            style.label_color
        };

        let source = match surface {
            PromptInputSurface::Controller => resolve_hint_key(glyphs, surface, entry.controller),
            PromptInputSurface::MouseOrKeyboard => Some(entry.keyboard.clone()),
        };

        let (icon_rect, cluster_w) = if let Some(before) = entry.label_before {
            let gap = gap_after_icon;
            let mut x = cluster_left;
            let before_draw_w = before_w.min((max_cluster_w - icon_px - gap * 2.0 - after_w).max(1.0));
            text_out.push(TextLabel {
                rect: [
                    x + style.label_pad_x,
                    label_top + style.label_pad_y,
                    before_draw_w.max(1.0),
                    legend_line_h,
                ],
                text: before.to_string(),
                color: label_color,
                font_px: Some(style.legend_font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
            x += before_draw_w + gap;
            let icon_rect = [x, iy, icon_px, icon_px];
            if let Some(source) = source.clone() {
                icon_cmds.push(ImageQuad {
                    inst: GpuInstance {
                        rect: icon_rect,
                        color: icon_tint,
                        user: 0,
                    },
                    source,
                });
            }
            x += icon_px + gap;
            let after_draw_w = after_w.min((max_cluster_w - (x - cluster_left)).max(1.0));
            text_out.push(TextLabel {
                rect: [
                    x + style.label_pad_x,
                    label_top + style.label_pad_y,
                    after_draw_w.max(1.0),
                    legend_line_h,
                ],
                text: entry.label.to_string(),
                color: label_color,
                font_px: Some(style.legend_font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
            let cluster_w = (x + after_draw_w + style.label_pad_x - cluster_left).max(1.0);
            (icon_rect, cluster_w)
        } else {
            let ix = cluster_left;
            let text_x = ix + icon_px + gap_after_icon;
            let max_text_w = (col_x + col_w - col_pad - text_x).max(10.0);
            let text_w = after_w.min(max_text_w).max(1.0);
            let icon_rect = [ix, iy, icon_px, icon_px];
            if let Some(source) = source {
                icon_cmds.push(ImageQuad {
                    inst: GpuInstance {
                        rect: icon_rect,
                        color: icon_tint,
                        user: 0,
                    },
                    source,
                });
            }
            text_out.push(TextLabel {
                rect: [
                    text_x + style.label_pad_x,
                    label_top + style.label_pad_y,
                    text_w,
                    legend_line_h,
                ],
                text: entry.label.to_string(),
                color: label_color,
                font_px: Some(style.legend_font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
            let cluster_w = icon_px + gap_after_icon + text_w + style.label_pad_x * 2.0;
            (icon_rect, cluster_w)
        };

        if let Some(pill_bg) = if entry.disabled {
            style.pill_bg_disabled
        } else {
            style.pill_bg
        } {
            let pill_left = cluster_left - pill_pad_x;
            let pill_w = (cluster_w + pill_pad_x * 2.0).max(1.0);
            pill_quads.push(GpuInstance {
                rect: [
                    pill_left,
                    label_top - pill_pad_y,
                    pill_w,
                    label_block_h + pill_pad_y * 2.0,
                ],
                color: pill_bg,
                user: 0,
            });
        }

        slots.push(ColumnHintSlot {
            column_index: col_i,
            icon_rect,
        });
    }

    if !pill_quads.is_empty() {
        frame.squircle_quads(pill_quads);
    }
    if !icon_cmds.is_empty() {
        frame.image_quads(icon_cmds);
    }

    slots
}
