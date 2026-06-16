//! Reusable Kenney controller / keyboard hint rows.
//!
//! All scenes use the same centred inline footer: `[icon]: verb · …` via [`HintStyle::standard`].

use crate::render::decal::{load_ui_font, measure_label_advances};
use crate::render::draw_cmd::{ImageQuad, ImageQuadSource, UiFrame};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::scenes::DrawCtx;
use crate::ui::glyph_source::{GlyphResolver, ShoulderSide, StickSide, TriggerSide};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::kenney_prompt_paths::keyboard_key;

const INLINE_SEP: &str = "   ·   ";
const INLINE_SLASH: &str = " / ";
const INLINE_PLUS: &str = "+";

const HINT_ICON_PX_MIN: f32 = 48.0;
const HINT_ICON_PX_MAX: f32 = 132.0;
const HINT_BAR_H_FRAC: f32 = 0.056;
const HINT_ICON_BAR_FRAC: f32 = 0.72;
/// Slightly smaller than the raw shop-legend reference (icons, labels, row height).
const HINT_METRICS_SCALE: f32 = 0.85;
/// Below 1080p, shrink hint icons (not labels) so dense rows fit without oversized keys.
const HINT_ICON_COMPACT_REF_H: f32 = 1080.0;
const HINT_ICON_COMPACT_FLOOR_H: f32 = 576.0;
const HINT_ICON_COMPACT_MIN_SCALE: f32 = 0.76;
const HINT_ROW_WIDTH_FRAC: f32 = 0.96;
const HINT_FIT_MIN_ICON_PX: f32 = 16.0;
const HINT_FIT_MIN_FONT_PX: f32 = 11.0;

#[inline]
fn hint_icon_compact_scale(window_h: f32) -> f32 {
    if window_h >= HINT_ICON_COMPACT_REF_H {
        return 1.0;
    }
    let t = ((window_h - HINT_ICON_COMPACT_FLOOR_H)
        / (HINT_ICON_COMPACT_REF_H - HINT_ICON_COMPACT_FLOOR_H))
        .clamp(0.0, 1.0);
    HINT_ICON_COMPACT_MIN_SCALE + t * (1.0 - HINT_ICON_COMPACT_MIN_SCALE)
}

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
    /// Height-proportional icon and label sizes; width fitting happens at draw time.
    pub fn primary(_window_w: f32, window_h: f32) -> Self {
        let s = HINT_METRICS_SCALE;
        let bar_h_ref = window_h * HINT_BAR_H_FRAC;
        let icon_px = (bar_h_ref * HINT_ICON_BAR_FRAC * 3.0)
            .clamp(HINT_ICON_PX_MIN, HINT_ICON_PX_MAX)
            * s
            * hint_icon_compact_scale(window_h);
        let legend_font_px = typography::size(typography::H24, window_h);
        let ui_font = load_ui_font();
        let legend_line_h = ui_font
            .as_ref()
            .and_then(|f| f.horizontal_line_metrics(legend_font_px))
            .map(|lm| lm.new_line_size)
            .unwrap_or(legend_font_px * 1.2)
            .max(legend_font_px * 0.85);
        let caption_px = typography::size(typography::H45, window_h);
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
    Dpad,
    Shoulder(ShoulderSide),
    SystemHelp,
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
        (PromptInputSurface::Controller, HintKey::Dpad) => glyphs.dpad_glyph(),
        (PromptInputSurface::Controller, HintKey::Shoulder(side)) => glyphs.shoulder_glyph(side),
        (PromptInputSurface::Controller, HintKey::SystemHelp) => glyphs.system_help_glyph(),
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

    pub fn bind_join(
        label: impl Into<String>,
        keys: Vec<HintKey>,
        within_join: HintKeyJoin,
    ) -> Self {
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

    /// Shared footer look for every scene.
    pub fn standard(window_w: f32, window_h: f32) -> Self {
        Self::from_metrics(
            HintMetrics::primary(window_w, window_h),
            [0.78, 0.80, 0.88, 0.92],
            color::alpha(color::PORCELAIN_AGED, 0.94),
        )
    }

    /// Scale [`standard`] metrics down so icons and labels fit a short inline band.
    pub fn fit_inline_rect(window_w: f32, window_h: f32, rect_h: f32) -> Self {
        let mut style = Self::standard(window_w, window_h);
        let cap = rect_h.max(10.0);
        let tallest = style.line_h.max(style.icon_px);
        if tallest > cap {
            let scale = cap / tallest;
            style.icon_px *= scale;
            style.gap_after_icon = style.icon_px * 0.18;
            style.font_px *= scale;
            style.line_h = cap;
        }
        style.icon_px = style.icon_px.min(cap);
        style
    }
}

fn inspect_exit_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Controller => {
            HintBind::alternatives("exit", vec![HintKey::Action(UiAction::NorthFacePress)])
        }
        InputMode::Cursor => HintBind::alternatives("exit", vec![HintKey::Keyboard("mouse_right")]),
        InputMode::Keyboard => {
            HintBind::alternatives("exit", vec![HintKey::Keyboard("keyboard_e")])
        }
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

/// Camera + exit controls while item inspect is active (item cycling uses normal focus nav).
pub fn inspect_camera_hint_row(input_mode: InputMode, show_preview: bool) -> Vec<HintSegment> {
    let mut row = HintRow::new()
        .push(inspect_orbit_bind(input_mode).into())
        .sep()
        .push(inspect_zoom_bind(input_mode).into());
    if show_preview {
        row = row.sep().push(confirm_bind(input_mode, "preview").into());
    }
    row.sep()
        .push(inspect_exit_bind(input_mode).into())
        .into_segments()
}

// ── Shared scene footer rows ─────────────────────────────────────────────────

fn back_bind(input_mode: InputMode) -> HintBind {
    HintBind::alternatives(
        "back",
        vec![HintKey::for_input(
            input_mode,
            UiAction::Cancel,
            "keyboard_escape",
        )],
    )
}

fn confirm_bind(input_mode: InputMode, label: impl Into<String>) -> HintBind {
    HintBind::alternatives(
        label,
        vec![HintKey::for_input(
            input_mode,
            UiAction::Confirm,
            "keyboard_return",
        )],
    )
}

fn navigate_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Controller => HintBind::alternatives("navigate", vec![HintKey::Dpad]),
        InputMode::Keyboard | InputMode::Cursor => {
            HintBind::alternatives("navigate", vec![HintKey::Keyboard("keyboard_arrows")])
        }
    }
}

fn scroll_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Controller => HintBind::alternatives("scroll", vec![HintKey::Dpad]),
        InputMode::Keyboard | InputMode::Cursor => HintBind::grouped(
            "scroll",
            vec![
                vec![HintKey::Keyboard("mouse_scroll")],
                vec![HintKey::Keyboard("keyboard_arrows_vertical")],
            ],
            HintKeyJoin::Slash,
        ),
    }
}

fn page_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Controller => HintBind::alternatives(
            "page",
            vec![
                HintKey::Shoulder(ShoulderSide::Left),
                HintKey::Shoulder(ShoulderSide::Right),
            ],
        ),
        InputMode::Keyboard | InputMode::Cursor => HintBind::alternatives(
            "page",
            vec![
                HintKey::Keyboard("keyboard_page_up"),
                HintKey::Keyboard("keyboard_page_down"),
            ],
        ),
    }
}

fn section_bind() -> HintBind {
    HintBind::alternatives(
        "section",
        vec![
            HintKey::Shoulder(ShoulderSide::Left),
            HintKey::Shoulder(ShoulderSide::Right),
        ],
    )
}

fn help_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Controller => HintBind::alternatives("guide", vec![HintKey::SystemHelp]),
        InputMode::Keyboard | InputMode::Cursor => {
            HintBind::alternatives("guide", vec![HintKey::Keyboard("keyboard_question")])
        }
    }
}

fn inspect_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Controller => {
            HintBind::alternatives("inspect", vec![HintKey::Action(UiAction::NorthFacePress)])
        }
        InputMode::Cursor => {
            HintBind::alternatives("inspect", vec![HintKey::Keyboard("mouse_right")])
        }
        InputMode::Keyboard => {
            HintBind::alternatives("inspect", vec![HintKey::Keyboard("keyboard_e")])
        }
    }
}

fn hold_sell_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Cursor => {
            HintBind::alternatives("hold sell", vec![HintKey::Keyboard("mouse_left")])
        }
        _ => HintBind::alternatives(
            "hold sell",
            vec![HintKey::for_input(
                input_mode,
                UiAction::WestFacePress,
                "keyboard_q",
            )],
        ),
    }
}

fn buy_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Cursor => HintBind::alternatives("buy", vec![HintKey::Keyboard("mouse_left")]),
        _ => HintBind::alternatives(
            "hold buy",
            vec![HintKey::for_input(
                input_mode,
                UiAction::Confirm,
                "keyboard_return",
            )],
        ),
    }
}

fn gameplay_select_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Cursor => {
            HintBind::alternatives("select", vec![HintKey::Keyboard("mouse_left")])
        }
        InputMode::Controller => {
            HintBind::alternatives("select", vec![HintKey::Action(UiAction::Confirm)])
        }
        InputMode::Keyboard => {
            HintBind::alternatives("select", vec![HintKey::Keyboard("keyboard_space")])
        }
    }
}

fn gameplay_deselect_bind(input_mode: InputMode) -> HintBind {
    HintBind::alternatives(
        "deselect",
        vec![HintKey::for_input(
            input_mode,
            UiAction::Cancel,
            "keyboard_backspace",
        )],
    )
}

fn gameplay_discard_bind(input_mode: InputMode) -> HintBind {
    HintBind::alternatives(
        "discard",
        vec![HintKey::for_input(
            input_mode,
            UiAction::WestFacePress,
            "keyboard_q",
        )],
    )
}

fn gameplay_play_bind(input_mode: InputMode) -> HintBind {
    HintBind::alternatives(
        "play",
        vec![HintKey::for_input(
            input_mode,
            UiAction::NorthFacePress,
            "keyboard_e",
        )],
    )
}

fn gameplay_cash_in_bind(input_mode: InputMode) -> HintBind {
    HintBind::alternatives(
        "cash in",
        vec![HintKey::for_input(
            input_mode,
            UiAction::TriggerStructure,
            "keyboard_t",
        )],
    )
}

/// Storeroom browse: optional buy / hold-to-sell, then inspect when focused.
pub fn shop_storeroom_footer_row(
    input_mode: InputMode,
    show_buy: bool,
    show_hold_sell: bool,
    show_inspect: bool,
) -> Vec<HintSegment> {
    let mut row = HintRow::new();
    if show_buy {
        row = row.push(buy_bind(input_mode).into());
        if show_hold_sell || show_inspect {
            row = row.sep();
        }
    }
    if show_hold_sell {
        row = row.push(hold_sell_bind(input_mode).into());
        if show_inspect {
            row = row.sep();
        }
    }
    if show_inspect {
        row = row.push(inspect_bind(input_mode).into());
    }
    row.into_segments()
}

/// Gameplay HUD footer: hand selection, available table actions (discard /
/// play / cash in), plus the guide hint — one centred row so they never overlap.
pub fn gameplay_footer_row(
    input_mode: InputMode,
    show_discard: bool,
    show_play: bool,
    show_cash_in: bool,
) -> Vec<HintSegment> {
    let mut row = HintRow::new()
        .push(gameplay_select_bind(input_mode).into())
        .sep()
        .push(gameplay_deselect_bind(input_mode).into());
    if show_discard {
        row = row.sep().push(gameplay_discard_bind(input_mode).into());
    }
    if show_play {
        row = row.sep().push(gameplay_play_bind(input_mode).into());
    }
    if show_cash_in {
        row = row.sep().push(gameplay_cash_in_bind(input_mode).into());
    }
    row.sep().push(help_bind(input_mode).into()).into_segments()
}

/// Hub / modal menus: move focus, then confirm the highlighted row.
pub fn menu_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    HintRow::new()
        .push(navigate_bind(input_mode).into())
        .sep()
        .push(confirm_bind(input_mode, "select").into())
        .into_segments()
}

/// Choose Your Tiles: spatial nav, LB/RB or Z/X to cycle tile material, then confirm.
pub fn tile_select_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    let mut row = HintRow::new().push(navigate_bind(input_mode).into());
    match input_mode {
        InputMode::Controller => {
            row = row.sep().push(
                HintBind::alternatives(
                    "tiles",
                    vec![
                        HintKey::Shoulder(ShoulderSide::Left),
                        HintKey::Shoulder(ShoulderSide::Right),
                    ],
                )
                .into(),
            );
        }
        InputMode::Keyboard => {
            row = row.sep().push(
                HintBind::alternatives(
                    "tiles",
                    vec![
                        HintKey::Keyboard("keyboard_z"),
                        HintKey::Keyboard("keyboard_x"),
                    ],
                )
                .into(),
            );
        }
        InputMode::Cursor => {}
    }
    row.sep()
        .push(confirm_bind(input_mode, "select").into())
        .into_segments()
}

/// Back + scroll affordances for read-only scroll panes.
pub fn back_scroll_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    HintRow::new()
        .push(back_bind(input_mode).into())
        .sep()
        .push(scroll_bind(input_mode).into())
        .into_segments()
}

/// Back-only footer for dev tools and simple overlays.
pub fn back_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    HintRow::new()
        .push(back_bind(input_mode).into())
        .into_segments()
}

fn count_step_bind(input_mode: InputMode) -> HintBind {
    match input_mode {
        InputMode::Controller => HintBind::alternatives(
            "±10",
            vec![
                HintKey::Shoulder(ShoulderSide::Left),
                HintKey::Shoulder(ShoulderSide::Right),
            ],
        ),
        InputMode::Keyboard | InputMode::Cursor => HintBind::alternatives(
            "±10",
            vec![
                HintKey::Keyboard("keyboard_bracket_left"),
                HintKey::Keyboard("keyboard_bracket_right"),
            ],
        ),
    }
}

fn focus_toggle_bind(input_mode: InputMode) -> HintBind {
    HintBind::alternatives(
        "focus",
        vec![HintKey::for_input(
            input_mode,
            UiAction::InvertSelection,
            "keyboard_z",
        )],
    )
}

/// Stairway prompt: ritual of decimation or pass unscathed.
pub fn stairway_prompt_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    HintRow::new()
        .push(navigate_bind(input_mode).into())
        .sep()
        .push(confirm_bind(input_mode, "select").into())
        .into_segments()
}

/// Decimation picker: spatial nav across tiles, suit sections, and footer.
pub fn decimation_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    let mut row = HintRow::new().push(navigate_bind(input_mode).into());
    match input_mode {
        InputMode::Controller => {
            row = row.sep().push(section_bind().into());
        }
        InputMode::Keyboard => {
            row = row.sep().push(
                HintBind::alternatives(
                    "section",
                    vec![
                        HintKey::Keyboard("keyboard_tab"),
                        HintKey::Keyboard("keyboard_bracket_left"),
                        HintKey::Keyboard("keyboard_bracket_right"),
                    ],
                )
                .into(),
            );
        }
        InputMode::Cursor => {}
    }
    row.sep()
        .push(confirm_bind(input_mode, "select").into())
        .sep()
        .push(back_bind(input_mode).into())
        .into_segments()
}

/// Tile stress lab: navigate tiles or footer, adjust tile count, confirm, back.
pub fn tile_stress_lab_footer_row(
    input_mode: InputMode,
    pickable: bool,
    tiles_focused: bool,
    controls_focused: bool,
) -> Vec<HintSegment> {
    let mut row = HintRow::new();
    if pickable && matches!(input_mode, InputMode::Controller | InputMode::Keyboard) {
        let nav_label = if tiles_focused { "tiles" } else { "footer" };
        row = row.push(match input_mode {
            InputMode::Controller => HintBind::alternatives(nav_label, vec![HintKey::Dpad]).into(),
            InputMode::Keyboard | InputMode::Cursor => {
                HintBind::alternatives(nav_label, vec![HintKey::Keyboard("keyboard_arrows")]).into()
            }
        });
        row = row.sep().push(focus_toggle_bind(input_mode).into());
    } else {
        row = row.push(navigate_bind(input_mode).into());
    }
    row = row.sep().push(count_step_bind(input_mode).into());
    let confirm_label = if controls_focused { "pick" } else { "select" };
    row.sep()
        .push(confirm_bind(input_mode, confirm_label).into())
        .sep()
        .push(back_bind(input_mode).into())
        .into_segments()
}

/// Guide / book chrome: back out, turn pages with shoulders or PgUp/PgDn.
pub fn guide_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    HintRow::new()
        .push(back_bind(input_mode).into())
        .sep()
        .push(page_bind(input_mode).into())
        .into_segments()
}

/// Chronicle ledger: back, pick a run (left), scroll stats (right).
pub fn chronicle_ledger_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    let select_run = match input_mode {
        InputMode::Controller => HintBind::grouped(
            "select run",
            vec![vec![HintKey::Dpad], vec![HintKey::Stick(StickSide::Left)]],
            HintKeyJoin::Slash,
        )
        .into(),
        InputMode::Keyboard | InputMode::Cursor => {
            HintBind::alternatives("select run", vec![HintKey::Keyboard("keyboard_wasd")]).into()
        }
    };
    let scroll = match input_mode {
        InputMode::Controller => {
            HintBind::alternatives("scroll stats", vec![HintKey::Stick(StickSide::Right)]).into()
        }
        InputMode::Keyboard | InputMode::Cursor => HintBind::grouped(
            "scroll stats",
            vec![
                vec![HintKey::Keyboard("mouse_scroll")],
                vec![HintKey::Keyboard("keyboard_arrows_vertical")],
            ],
            HintKeyJoin::Slash,
        )
        .into(),
    };
    HintRow::new()
        .push(back_bind(input_mode).into())
        .sep()
        .push(confirm_bind(input_mode, "select").into())
        .sep()
        .push(select_run)
        .sep()
        .push(scroll)
        .into_segments()
}

/// Wall ledger: back, select tile (WASD / d-pad / left stick), scroll summary (↑↓ / wheel / right stick).
pub fn wall_ledger_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    let select_tile = match input_mode {
        InputMode::Controller => HintBind::grouped(
            "select tile",
            vec![vec![HintKey::Dpad], vec![HintKey::Stick(StickSide::Left)]],
            HintKeyJoin::Slash,
        )
        .into(),
        InputMode::Keyboard | InputMode::Cursor => {
            HintBind::alternatives("select tile", vec![HintKey::Keyboard("keyboard_wasd")]).into()
        }
    };
    let scroll = match input_mode {
        InputMode::Controller => {
            HintBind::alternatives("scroll", vec![HintKey::Stick(StickSide::Right)]).into()
        }
        InputMode::Keyboard | InputMode::Cursor => HintBind::grouped(
            "scroll",
            vec![
                vec![HintKey::Keyboard("mouse_scroll")],
                vec![HintKey::Keyboard("keyboard_arrows_vertical")],
            ],
            HintKeyJoin::Slash,
        )
        .into(),
    };
    HintRow::new()
        .push(
            HintBind::alternatives(
                "back",
                vec![HintKey::for_input(
                    input_mode,
                    UiAction::Cancel,
                    "keyboard_escape",
                )],
            )
            .into(),
        )
        .sep()
        .push(
            HintBind::alternatives(
                "select",
                vec![HintKey::for_input(
                    input_mode,
                    UiAction::Confirm,
                    "keyboard_return",
                )],
            )
            .into(),
        )
        .sep()
        .push(select_tile)
        .sep()
        .push(scroll)
        .into_segments()
}

/// Yaku journal screen footer: pick a row, page the catalog.
pub fn journal_plaque_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    HintRow::new()
        .push(navigate_bind(input_mode).into())
        .sep()
        .push(page_bind(input_mode).into())
        .into_segments()
}

/// Archive grid (not in item inspect): browse, cycle sections / pages, preview, inspect.
pub fn archive_browse_footer_row(
    input_mode: InputMode,
    multi_page: bool,
    show_preview: bool,
    show_inspect: bool,
) -> Vec<HintSegment> {
    let mut row = HintRow::new().push(navigate_bind(input_mode).into());
    match input_mode {
        InputMode::Controller => {
            row = row.sep().push(section_bind().into());
        }
        InputMode::Keyboard | InputMode::Cursor if multi_page => {
            row = row.sep().push(page_bind(input_mode).into());
        }
        InputMode::Keyboard | InputMode::Cursor => {}
    }
    if show_preview {
        row = row.sep().push(confirm_bind(input_mode, "preview").into());
    }
    if show_inspect {
        row = row.sep().push(inspect_bind(input_mode).into());
    }
    row.into_segments()
}

/// Run-end and celebration overlays: optional flavor copy + confirm to continue.
pub fn confirm_continue_footer_row(input_mode: InputMode, flavor: &str) -> Vec<HintSegment> {
    let mut row = HintRow::new();
    if !flavor.is_empty() {
        row = row.push(HintSegment::text(flavor.to_string()));
        row = row.sep();
    }
    row.push(confirm_bind(input_mode, "continue").into())
        .into_segments()
}

/// Single confirm affordance (e.g. unseal pack).
pub fn confirm_action_footer_row(input_mode: InputMode, action_label: &str) -> Vec<HintSegment> {
    HintRow::new()
        .push(confirm_bind(input_mode, action_label).into())
        .into_segments()
}

/// Options screen: move rows, scroll the panel, go back.
pub fn options_footer_row(input_mode: InputMode) -> Vec<HintSegment> {
    HintRow::new()
        .push(navigate_bind(input_mode).into())
        .sep()
        .push(scroll_bind(input_mode).into())
        .sep()
        .push(back_bind(input_mode).into())
        .into_segments()
}

const SCREEN_FOOTER_BOTTOM_FRAC: f32 = 0.018;
const SCREEN_FOOTER_FADE_FRAC: f32 = 0.10;

/// Darkens the bottom of the screen so footer hint labels stay readable over busy 3D scenes.
pub fn push_screen_footer_fade(frame: &mut UiFrame, window_w: f32, window_h: f32) {
    let band = window_h * SCREEN_FOOTER_FADE_FRAC;
    if band < 2.0 {
        return;
    }
    let y0 = window_h - band;
    const STRIPS: usize = 32;
    let strip_h = band / STRIPS as f32;
    let mut quads = Vec::with_capacity(STRIPS);
    for i in 0..STRIPS {
        let t0 = i as f32 / STRIPS as f32;
        let t1 = (i + 1) as f32 / STRIPS as f32;
        let t_mid = (t0 + t1) * 0.5;
        let alpha = t_mid;
        if alpha <= 0.001 {
            continue;
        }
        quads.push(GpuInstance {
            rect: [0.0, y0 + strip_h * i as f32, window_w, strip_h + 0.5],
            color: [0.0, 0.0, 0.0, alpha],
            user: 0,
        });
    }
    if !quads.is_empty() {
        frame.overlay_quads(quads);
    }
}

/// Vertical space to leave clear at the bottom when using [`push_screen_footer_hint`].
pub fn screen_footer_reserve(window_w: f32, window_h: f32) -> f32 {
    HintStyle::standard(window_w, window_h).line_h + window_h * SCREEN_FOOTER_BOTTOM_FRAC
}

/// Push a single centred footer row above the bottom edge.
pub fn push_screen_footer_hint(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    row: Vec<HintSegment>,
    style: HintStyle,
) -> Vec<InlineHintIconSlot> {
    push_screen_footer_hint_for(
        frame,
        ctx.layout.window_w,
        ctx.layout.window_h,
        ctx.input_mode,
        ctx.glyphs,
        row,
        style,
    )
}

/// Like [`push_screen_footer_hint`] when [`DrawCtx`] was already consumed.
pub fn push_screen_footer_hint_for(
    frame: &mut UiFrame,
    window_w: f32,
    window_h: f32,
    input_mode: InputMode,
    glyphs: GlyphResolver,
    row: Vec<HintSegment>,
    style: HintStyle,
) -> Vec<InlineHintIconSlot> {
    push_screen_footer_fade(frame, window_w, window_h);
    let line_h = style.line_h;
    let y = window_h - line_h - window_h * SCREEN_FOOTER_BOTTOM_FRAC;
    push_inline_hint_rows_for(
        frame,
        input_mode,
        glyphs,
        &[[0.0, y, window_w, line_h]],
        &[row],
        style,
    )
}

/// Y coordinate of the top edge of a [`push_screen_footer_hint`] row.
pub fn screen_footer_top(window_h: f32, style: HintStyle) -> f32 {
    window_h - style.line_h - window_h * SCREEN_FOOTER_BOTTOM_FRAC
}

/// Apply alpha scaling to hint colours (celebration overlays).
pub fn hint_style_with_alpha(mut style: HintStyle, alpha: f32) -> HintStyle {
    let a = alpha.clamp(0.0, 1.0);
    style.text_color[3] *= a;
    style.icon_tint[3] *= a;
    style
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

fn for_each_inline_bind_part(bind: &HintBind, mut f: impl FnMut(InlineBindPart)) {
    let within_text = key_join_text(bind.within_join);
    for (gi, group) in bind.key_groups.iter().enumerate() {
        if gi > 0 {
            f(InlineBindPart::GroupSep);
        }
        for (ki, &key) in group.iter().enumerate() {
            if ki > 0 && !within_text.is_empty() {
                f(InlineBindPart::WithinSep);
            }
            let last_icon = gi + 1 == bind.key_groups.len() && ki + 1 == group.len();
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
    style: HintStyle,
    segments: &[InlineSegmentRef<'_>],
) -> f32 {
    segments.iter().fold(0.0, |acc, seg| {
        acc + match *seg {
            InlineSegmentRef::Sep => measure_text(style.font_px, INLINE_SEP),
            InlineSegmentRef::PlainText(text) => measure_text(style.font_px, text),
            InlineSegmentRef::Bind(bind) => measure_inline_bind(
                glyphs,
                surface,
                style.icon_px,
                style.gap_after_icon,
                style.font_px,
                bind,
            ),
        }
    })
}

fn shrink_hint_style_to_fit_width(
    mut style: HintStyle,
    glyphs: GlyphResolver,
    surface: PromptInputSurface,
    rows: &[Vec<HintSegment>],
    max_row_w: f32,
) -> HintStyle {
    let max_w = max_row_w.max(1.0);
    loop {
        let fits = rows.iter().all(|row| {
            let mut scratch: Vec<InlineSegmentRef<'_>> = Vec::new();
            let segs = inline_segment_refs(&mut scratch, row);
            measure_inline_row(glyphs, surface, style, segs) <= max_w
        });
        if fits {
            return style;
        }
        if style.icon_px <= HINT_FIT_MIN_ICON_PX && style.font_px <= HINT_FIT_MIN_FONT_PX {
            return style;
        }
        style.icon_px = (style.icon_px * 0.94).max(HINT_FIT_MIN_ICON_PX);
        style.font_px = (style.font_px * 0.94).max(HINT_FIT_MIN_FONT_PX);
        style.gap_after_icon = style.icon_px * 0.18;
        style.line_h = style
            .line_h
            .min(style.icon_px * 1.06)
            .max(style.font_px * 1.05);
    }
}

/// Icon geometry emitted for one bind key in an inline hint row (e.g. hold-progress rings).
#[derive(Clone, Copy, Debug)]
pub struct InlineHintIconSlot {
    pub key: HintKey,
    pub icon_rect: [f32; 4],
}

/// Whether `key` is the face/key glyph for shop hold-to-sell (West / Q / mouse hold).
pub fn is_hold_sell_hint_key(key: HintKey) -> bool {
    matches!(
        key,
        HintKey::Action(UiAction::WestFacePress)
            | HintKey::Keyboard("keyboard_q")
            | HintKey::Keyboard("mouse_left")
    )
}

/// Whether `key` is the face/key glyph for shop buy (Confirm / Enter / click).
pub fn is_shop_buy_hint_key(key: HintKey) -> bool {
    matches!(
        key,
        HintKey::Action(UiAction::Confirm)
            | HintKey::Keyboard("keyboard_return")
            | HintKey::Keyboard("mouse_left")
    )
}

/// Whether `key` is the face/key glyph for confirm (South / Enter / Space).
pub fn is_confirm_hint_key(key: HintKey) -> bool {
    matches!(
        key,
        HintKey::Action(UiAction::Confirm)
            | HintKey::Keyboard("keyboard_return")
            | HintKey::Keyboard("keyboard_space")
    )
}

/// Whether `key` is the face/key glyph for gameplay hold-to-cash-in (Trigger / T).
pub fn is_cash_in_hint_key(key: HintKey) -> bool {
    matches!(
        key,
        HintKey::Action(UiAction::TriggerStructure) | HintKey::Keyboard("keyboard_t")
    )
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
) -> Vec<InlineHintIconSlot> {
    let mut slots = Vec::new();
    let [rx, ry, rw, rh] = rect;
    let icon_px = icon_px.min(rh);
    let line_h = style.line_h.min(rh);
    let row_w = measure_inline_row(glyphs, surface, style, segments);
    let mut x = rx + (rw - row_w).max(0.0) * 0.5;
    let iy = ry + (rh - icon_px).max(0.0) * 0.5;
    let text_y = ry + (rh - line_h).max(0.0) * 0.5;

    for seg in segments {
        match *seg {
            InlineSegmentRef::Sep => {
                let w = measure_text(style.font_px, INLINE_SEP);
                push_outlined_inline_text_label(texts, [x, text_y, w, line_h], INLINE_SEP, style);
                x += w;
            }
            InlineSegmentRef::PlainText(text) => {
                let w = measure_text(style.font_px, text);
                push_outlined_inline_text_label(
                    texts,
                    [x, text_y, w.max(1.0), line_h],
                    text,
                    style,
                );
                x += w.max(1.0);
            }
            InlineSegmentRef::Bind(bind) => {
                let within_text = key_join_text(bind.within_join);
                for_each_inline_bind_part(bind, |part| match part {
                    InlineBindPart::GroupSep => {
                        let w = measure_text(style.font_px, INLINE_SLASH);
                        push_outlined_inline_text_label(
                            texts,
                            [x, text_y, w, line_h],
                            INLINE_SLASH,
                            style,
                        );
                        x += w;
                    }
                    InlineBindPart::WithinSep => {
                        let w = measure_text(style.font_px, within_text);
                        push_outlined_inline_text_label(
                            texts,
                            [x, text_y, w, line_h],
                            within_text,
                            style,
                        );
                        x += w;
                    }
                    InlineBindPart::Key(key, last_icon) => {
                        if let Some(source) = resolve_hint_key(glyphs, surface, key) {
                            let icon_rect = [x, iy, icon_px, icon_px];
                            icon_cmds.push(ImageQuad {
                                inst: GpuInstance {
                                    rect: icon_rect,
                                    color: style.icon_tint,
                                    user: 0,
                                },
                                source,
                                clip_rect: None,
                            });
                            slots.push(InlineHintIconSlot { key, icon_rect });
                            x += icon_px;
                            if last_icon {
                                x += gap_after_icon;
                            }
                        }
                    }
                });
                let suffix = bind_suffix(&bind.label);
                let w = measure_text(style.font_px, &suffix);
                push_outlined_inline_text_label(
                    texts,
                    [x, text_y, w.max(1.0), line_h],
                    &suffix,
                    style,
                );
                x += w.max(1.0);
            }
        }
    }
    slots
}

/// Push one or more centred inline hint rows.
pub fn push_inline_hint_rows(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    row_rects: &[[f32; 4]],
    rows: &[Vec<HintSegment>],
    style: HintStyle,
) -> Vec<InlineHintIconSlot> {
    push_inline_hint_rows_for(frame, ctx.input_mode, ctx.glyphs, row_rects, rows, style)
}

/// Like [`push_inline_hint_rows`] when [`DrawCtx`] was already consumed.
pub fn push_inline_hint_rows_for(
    frame: &mut UiFrame,
    input_mode: InputMode,
    glyphs: GlyphResolver,
    row_rects: &[[f32; 4]],
    rows: &[Vec<HintSegment>],
    style: HintStyle,
) -> Vec<InlineHintIconSlot> {
    if row_rects.is_empty() || rows.is_empty() || row_rects.len() != rows.len() {
        return Vec::new();
    }

    let surface = prompt_surface(input_mode);
    let max_row_w = row_rects
        .iter()
        .map(|r| r[2])
        .fold(0.0_f32, f32::max)
        .max(1.0)
        * HINT_ROW_WIDTH_FRAC;
    let max_row_h = row_rects
        .iter()
        .map(|r| r[3])
        .fold(0.0_f32, f32::max)
        .max(1.0);
    let mut style = style;
    style.icon_px = style.icon_px.min(max_row_h);
    style.line_h = style.line_h.min(max_row_h);
    style.gap_after_icon = style.gap_after_icon.min(style.icon_px * 0.18);
    style = shrink_hint_style_to_fit_width(style, glyphs, surface, rows, max_row_w);
    let icon_px = style.icon_px;
    let gap_after_icon = style.gap_after_icon;

    let mut icon_cmds: Vec<ImageQuad> = Vec::new();
    let mut texts: Vec<TextLabel> = Vec::new();
    let mut slots = Vec::new();

    for (row, rect) in rows.iter().zip(row_rects.iter()) {
        let mut scratch: Vec<InlineSegmentRef<'_>> = Vec::new();
        let segs = inline_segment_refs(&mut scratch, row);
        slots.extend(emit_inline_row(
            glyphs,
            surface,
            icon_px,
            gap_after_icon,
            style,
            *rect,
            segs,
            &mut icon_cmds,
            &mut texts,
        ));
    }

    if !icon_cmds.is_empty() {
        frame.image_quads(icon_cmds);
    }
    if !texts.is_empty() {
        frame.texts(texts);
    }
    slots
}

fn hint_text_outline_offsets(font_px: f32) -> [(f32, f32); 8] {
    let d = (font_px * 0.055).clamp(1.0, 2.5);
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

fn push_outlined_inline_text_label(
    texts: &mut Vec<TextLabel>,
    rect: [f32; 4],
    text: &str,
    style: HintStyle,
) {
    let label = inline_text_label(rect, text, style);
    let outline = [0.0, 0.0, 0.0, style.text_color[3].min(0.95)];
    for (dx, dy) in hint_text_outline_offsets(style.font_px) {
        let mut stroke = label.clone();
        stroke.rect[0] += dx;
        stroke.rect[1] += dy;
        stroke.color = outline;
        texts.push(stroke);
    }
    texts.push(label);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::button_prompts::GamepadStyle;
    use crate::ui::input::InputMode;

    #[test]
    fn labels_scale_with_height_icons_compact_below_1080p() {
        let tall = HintStyle::standard(1920.0, 1080.0);
        let short = HintStyle::standard(1024.0, 576.0);
        let tall_font_ratio = tall.font_px / 1080.0;
        let short_font_ratio = short.font_px / 576.0;
        assert!(
            (tall_font_ratio - short_font_ratio).abs() < 0.002,
            "label font/window_h should stay constant"
        );
        assert!(
            short.icon_px < tall.icon_px * (576.0 / 1080.0),
            "576p icons should be smaller than pure height scaling"
        );
        assert!(
            (hint_icon_compact_scale(576.0) - HINT_ICON_COMPACT_MIN_SCALE).abs() < f32::EPSILON
        );
    }

    #[test]
    fn gameplay_footer_fits_common_resolutions() {
        let glyphs = GlyphResolver::new(GamepadStyle::Xbox, false, false);
        let row = gameplay_footer_row(InputMode::Cursor, true, true, true);
        for (w, h) in [(1920.0, 1080.0), (1280.0, 720.0), (1024.0, 576.0)] {
            let max_w = w * HINT_ROW_WIDTH_FRAC;
            let style = shrink_hint_style_to_fit_width(
                HintStyle::standard(w, h),
                glyphs,
                PromptInputSurface::MouseOrKeyboard,
                std::slice::from_ref(&row),
                max_w,
            );
            let mut scratch = Vec::new();
            let segs = inline_segment_refs(&mut scratch, &row);
            let width =
                measure_inline_row(glyphs, PromptInputSurface::MouseOrKeyboard, style, segs);
            assert!(
                width <= max_w + 0.5,
                "gameplay footer overflow at {w}x{h}: {width:.1}px > {max_w:.1}px"
            );
        }
    }
}
