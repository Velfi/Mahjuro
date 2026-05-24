//! Options scene — volume sliders, visual settings, rendering, and accessibility.
//!
//! Layout: a table-of-contents (TOC) column on the left links to
//! sections (Audio, Graphics, Controls, Accessibility, Data) in a
//! scrollable content pane on the right.

use crate::audio::SfxId;
use crate::game::event_bus::GameEvent;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::clip::intersect_rect;
use crate::ui::button_prompts::{ButtonPrompt, GamepadStyle};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::smooth_scroll::SmoothScroll;

use crate::render::draw_cmd::UiFrame;

use super::main_menu_exterior::MainMenuExteriorScene;
use super::profile_select::ProfileSelectScene;
use super::{ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

// ── Constants ──────────────────────────────────────────────────────────

/// Volume adjustment step per input press.
const VOL_STEP: f32 = 0.05;
/// Gamma adjustment step per input press.
const GAMMA_STEP: f32 = 0.05;

/// Click-id base for TOC links (high range to avoid collisions).
const TOC_ID_BASE: u32 = 0xF200;
/// Click-id for the fixed Back button.
const BACK_ID: u32 = 0xF210;

/// Label column width as a fraction of the content pane.
const LABEL_FRAC: f32 = 0.42;
/// Control column width (slider track or value chip).
const CONTROL_FRAC: f32 = 0.48;
/// Trailing readout column for slider percentages.
const READOUT_FRAC: f32 = 0.10;

// ── Sections ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Section {
    Audio,
    Graphics,
    Controls,
    Accessibility,
    Data,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Section::Audio => "Audio",
            Section::Graphics => "Graphics",
            Section::Controls => "Controls",
            Section::Accessibility => "Accessibility",
            Section::Data => "Data",
        }
    }
}

const SECTIONS: &[Section] = &[
    Section::Audio,
    Section::Graphics,
    Section::Controls,
    Section::Accessibility,
    Section::Data,
];

// ── Rows ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Row {
    Master,
    Music,
    Sfx,
    SfxToggle,
    Gamma,
    Effects,
    Tile,
    Tileset,
    Surface,
    BorderlessFullscreen,
    Shadows,
    Ssr,
    Hdr,
    UndoDiscard,
    SwapAb,
    SwapXy,
    XyQuickAction,
    HoldToSellRumble,
    AutoCashInOnFullStructure,
    Hints,
    GlyphPrompts,
    ExportPlayStats,
    SaveAndProfiles,
}

impl Row {
    fn click_id(self) -> u32 {
        self as u32 + 1
    }

    fn from_click_id(id: u32) -> Option<Row> {
        ROWS.iter().copied().find(|r| r.click_id() == id)
    }

    fn is_slider(self) -> bool {
        matches!(self, Row::Master | Row::Music | Row::Sfx | Row::Gamma)
    }
}

/// Navigable rows in section order (keyboard Up/Down cycles through these).
const ROWS: &[Row] = &[
    Row::Master,
    Row::Music,
    Row::Sfx,
    Row::SfxToggle,
    Row::Gamma,
    Row::Effects,
    Row::Tile,
    Row::Tileset,
    Row::Surface,
    Row::BorderlessFullscreen,
    Row::Shadows,
    Row::Ssr,
    Row::Hdr,
    Row::SwapAb,
    Row::SwapXy,
    Row::XyQuickAction,
    Row::HoldToSellRumble,
    Row::AutoCashInOnFullStructure,
    Row::UndoDiscard,
    Row::Hints,
    Row::GlyphPrompts,
    Row::ExportPlayStats,
    Row::SaveAndProfiles,
];

// ── Content slots (section headers interspersed with rows) ─────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContentSlot {
    Header(Section),
    Row(Row),
}

const CONTENT: &[ContentSlot] = &[
    ContentSlot::Header(Section::Audio),
    ContentSlot::Row(Row::Master),
    ContentSlot::Row(Row::Music),
    ContentSlot::Row(Row::Sfx),
    ContentSlot::Row(Row::SfxToggle),
    ContentSlot::Header(Section::Graphics),
    ContentSlot::Row(Row::Gamma),
    ContentSlot::Row(Row::Effects),
    ContentSlot::Row(Row::Tile),
    ContentSlot::Row(Row::Tileset),
    ContentSlot::Row(Row::Surface),
    ContentSlot::Row(Row::BorderlessFullscreen),
    ContentSlot::Row(Row::Shadows),
    ContentSlot::Row(Row::Ssr),
    ContentSlot::Row(Row::Hdr),
    ContentSlot::Header(Section::Controls),
    ContentSlot::Row(Row::SwapAb),
    ContentSlot::Row(Row::SwapXy),
    ContentSlot::Row(Row::XyQuickAction),
    ContentSlot::Row(Row::HoldToSellRumble),
    ContentSlot::Row(Row::AutoCashInOnFullStructure),
    ContentSlot::Header(Section::Accessibility),
    ContentSlot::Row(Row::UndoDiscard),
    ContentSlot::Row(Row::Hints),
    ContentSlot::Row(Row::GlyphPrompts),
    ContentSlot::Header(Section::Data),
    ContentSlot::Row(Row::ExportPlayStats),
    ContentSlot::Row(Row::SaveAndProfiles),
];

fn content_index_of_row(row: Row) -> usize {
    CONTENT
        .iter()
        .position(|s| matches!(s, ContentSlot::Row(r) if *r == row))
        .unwrap()
}

fn content_index_of_section(section: Section) -> usize {
    CONTENT
        .iter()
        .position(|s| matches!(s, ContentSlot::Header(sec) if *sec == section))
        .unwrap()
}

/// Which section header is at the top of the visible content viewport?
fn section_at_scroll(scroll: f32) -> Section {
    let idx = scroll.floor() as usize;
    for slot in CONTENT.get(idx..).into_iter().flatten() {
        if let ContentSlot::Header(sec) = slot {
            return *sec;
        }
    }
    for slot in CONTENT[..idx.min(CONTENT.len().saturating_sub(1)) + 1]
        .iter()
        .rev()
    {
        if let ContentSlot::Header(sec) = slot {
            return *sec;
        }
    }
    Section::Audio
}

/// Input/device context for hint copy and scroll fades in [`OptionsScene::draw_overlay`].
pub struct OptionsDrawHint {
    pub input_mode: InputMode,
    pub gamepad_style: GamepadStyle,
    pub swap_ab: bool,
    pub swap_xy: bool,
    /// Color the top/bottom scroll fades blend into (standalone vs pause overlay).
    pub scroll_fade_backdrop: [f32; 4],
}

impl Default for OptionsDrawHint {
    fn default() -> Self {
        Self {
            input_mode: InputMode::Cursor,
            gamepad_style: GamepadStyle::default(),
            swap_ab: false,
            swap_xy: false,
            scroll_fade_backdrop: color::WALNUT_INK,
        }
    }
}

impl OptionsDrawHint {
    pub fn from_draw_ctx(ctx: &DrawCtx<'_>) -> Self {
        Self {
            input_mode: ctx.input_mode,
            gamepad_style: ctx.gamepad_style,
            swap_ab: ctx.gamepad_swap_ab,
            swap_xy: ctx.gamepad_swap_xy,
            scroll_fade_backdrop: color::WALNUT_INK,
        }
    }

    /// Pause-menu overlay sits on a dim walnut scrim, not a solid fill.
    pub fn pause_overlay(ctx: &DrawCtx<'_>) -> Self {
        Self {
            scroll_fade_backdrop: color::alpha(color::WALNUT_INK, 0.78),
            ..Self::from_draw_ctx(ctx)
        }
    }
}

struct RowColumns {
    label_x: f32,
    label_w: f32,
    control_x: f32,
    control_w: f32,
    readout_x: f32,
    readout_w: f32,
}

fn row_columns(content_x: f32, content_w: f32) -> RowColumns {
    let label_w = content_w * LABEL_FRAC;
    let control_w = content_w * CONTROL_FRAC;
    let readout_w = content_w * READOUT_FRAC;
    RowColumns {
        label_x: content_x,
        label_w,
        control_x: content_x + label_w,
        control_w,
        readout_x: content_x + label_w + control_w,
        readout_w,
    }
}

fn on_off(enabled: bool) -> String {
    if enabled {
        "ON".into()
    } else {
        "OFF".into()
    }
}

fn row_copy(row: Row, scene: &OptionsScene) -> (&'static str, String) {
    match row {
        Row::Master => ("Master", format!("{}%", (scene.master_volume * 100.0).round() as u32)),
        Row::Music => ("Music", format!("{}%", (scene.music_volume * 100.0).round() as u32)),
        Row::Sfx => ("SFX", format!("{}%", (scene.sfx_volume * 100.0).round() as u32)),
        Row::SfxToggle => ("SFX enabled", on_off(scene.sfx_enabled)),
        Row::Gamma => ("Gamma", format!("{:.2}", scene.gamma)),
        Row::Effects => ("Effects quality", scene.effects_quality.label().into()),
        Row::Tile => ("Tile style", scene.tile_preset.label().into()),
        Row::Tileset => ("Tile set", scene.tileset_name.clone()),
        Row::Surface => ("Surface", scene.surface_kind.label().into()),
        Row::BorderlessFullscreen => (
            "Window mode",
            if scene.borderless_fullscreen {
                "Borderless".into()
            } else {
                "Windowed".into()
            },
        ),
        Row::Shadows => ("Shadows", on_off(scene.shadows_enabled)),
        Row::Ssr => ("Reflections", on_off(scene.ssr_enabled)),
        Row::Hdr => ("HDR", on_off(scene.hdr_enabled)),
        Row::SwapAb => ("Swap A/B", on_off(scene.swap_ab)),
        Row::SwapXy => ("Swap X/Y", on_off(scene.swap_xy)),
        Row::XyQuickAction => ("Face buttons: play/discard", on_off(scene.xy_quick_action)),
        Row::HoldToSellRumble => ("Controller rumble", on_off(scene.hold_to_sell_rumble)),
        Row::AutoCashInOnFullStructure => (
            "Auto cash-in",
            on_off(scene.auto_cash_in_on_full_structure),
        ),
        Row::Hints => ("Hints", on_off(scene.hints_enabled)),
        Row::GlyphPrompts => ("Button glyphs", scene.glyph_prompt.label().into()),
        Row::UndoDiscard => ("Discard undo", on_off(scene.discard_undo_enabled)),
        Row::ExportPlayStats => ("Export play stats", String::new()),
        Row::SaveAndProfiles => ("Save & profiles", String::new()),
    }
}

// ── Layout ─────────────────────────────────────────────────────────────

struct PanelLayout {
    scale: f32,
    // Title
    title_y: f32,
    title_h: f32,
    // TOC column
    toc_x: f32,
    toc_w: f32,
    toc_start_y: f32,
    toc_item_h: f32,
    toc_gap: f32,
    // Content column
    content_x: f32,
    content_w: f32,
    content_start_y: f32,
    // Slot sizing
    slot_h: f32,
    slot_gap: f32,
    visible_slots: usize,
    // Back button
    back_x: f32,
    back_y: f32,
    back_w: f32,
    back_h: f32,
    // Hint
    hint_y: f32,
    hint_h: f32,
    // Version footer (below hint)
    version_y: f32,
    version_h: f32,
}

fn compute_layout(w: f32, h: f32) -> PanelLayout {
    let scale = (w.min(h)) / 600.0;

    let title_h = (48.0 * scale).max(28.0);
    let title_y = h * 0.06;

    // Two-column split: TOC on the left, scrollable content on the right.
    let margin = w * 0.06;
    let total_w = w - margin * 2.0;
    let toc_w = (total_w * 0.18).min(160.0 * scale).max(80.0);
    let col_gap = (20.0 * scale).max(10.0);
    let content_w = total_w - toc_w - col_gap;
    let toc_x = margin;
    let content_x = margin + toc_w + col_gap;

    let body_top = title_y + title_h + h * 0.04;
    let toc_start_y = body_top;
    let toc_item_h = (36.0 * scale).max(24.0);
    let toc_gap = (8.0 * scale).max(4.0);

    let content_start_y = body_top;

    let slot_h = (40.0 * scale).max(26.0);
    let slot_gap = (10.0 * scale).max(5.0);

    // Back button, hint, and version footer pinned to the bottom.
    let back_h = (42.0 * scale).max(28.0);
    let hint_h = (20.0 * scale).max(14.0);
    let version_h = (14.0 * scale).max(10.0);
    let version_y = h - version_h - (4.0 * scale);
    let hint_y = version_y - hint_h - (6.0 * scale);
    let back_y = hint_y - back_h - (12.0 * scale);
    let back_w = total_w;
    let back_x = margin;

    let content_end_y = back_y - (12.0 * scale);
    let slot_step = slot_h + slot_gap;
    let avail_h = (content_end_y - content_start_y).max(slot_step);
    let visible_slots = ((avail_h / slot_step).floor() as usize).max(1);

    PanelLayout {
        scale,
        title_y,
        title_h,
        toc_x,
        toc_w,
        toc_start_y,
        toc_item_h,
        toc_gap,
        content_x,
        content_w,
        content_start_y,
        slot_h,
        slot_gap,
        visible_slots,
        back_x,
        back_y,
        back_w,
        back_h,
        hint_y,
        hint_h,
        version_y,
        version_h,
    }
}

/// Frame input for [`OptionsScene::update_input`].
pub struct OptionsInput<'a> {
    pub actions: &'a [UiAction],
    pub button_clicks: &'a [u32],
    pub cursor_pos: (f32, f32),
    pub window_w: f32,
    pub window_h: f32,
    pub scroll_lines: f32,
    pub input_mode: InputMode,
    pub mouse_left_down: bool,
}

// ── OptionsScene ───────────────────────────────────────────────────────

pub struct OptionsScene {
    focused: Row,
    /// When true the Back button (below the scroll area) has keyboard focus.
    back_focused: bool,
    /// Latched when user input changes focus to a different row/back button.
    focus_changed: bool,
    confirm_requested: bool,
    cancel_requested: bool,
    /// User activated "Export play stats" this frame (after [`Self::update_input`]).
    export_requested: bool,
    /// User chose Save & profiles — open profile picker then return here.
    profile_select_requested: bool,
    /// Smooth-scrolling state for the content pane.
    scroll: SmoothScroll,
    /// While `Some`, LMB is held after pressing on this row's slider track.
    dragging_slider: Option<Row>,

    /// Local copy of settings; written back on change and scene exit.
    pub master_volume: f32,
    pub sfx_volume: f32,
    pub music_volume: f32,
    pub sfx_enabled: bool,
    pub effects_quality: crate::persistence::EffectsQuality,
    pub tile_preset: crate::persistence::TilePreset,
    pub tileset_name: String,
    pub available_tilesets: Vec<String>,
    pub surface_kind: crate::persistence::SurfaceKind,
    pub gamma: f32,
    pub shadows_enabled: bool,
    pub ssr_enabled: bool,
    pub hdr_enabled: bool,
    pub borderless_fullscreen: bool,
    pub swap_ab: bool,
    pub swap_xy: bool,
    /// Mirrors `AppSettings::controller_layout_user_set`. Goes ON the moment
    /// the player toggles `swap_ab` or `swap_xy` here, locking out the
    /// auto-applied controller-style defaults from then on.
    pub controller_layout_user_set: bool,
    pub xy_quick_action: bool,
    pub hold_to_sell_rumble: bool,
    pub auto_cash_in_on_full_structure: bool,
    pub hints_enabled: bool,
    pub discard_undo_enabled: bool,
    pub glyph_prompt: crate::persistence::GlyphPromptSetting,
}

impl OptionsScene {
    pub fn new() -> Self {
        let settings = crate::persistence::load_settings();
        let mut available_tilesets = crate::asset_path::list_player_tilesets();
        if available_tilesets.is_empty() {
            available_tilesets.push("original".to_string());
        }
        // Guarantee the persisted choice is still valid; fall back to the first
        // available set if the folder was removed.
        let tileset_name = if available_tilesets.contains(&settings.tileset_name) {
            settings.tileset_name.clone()
        } else {
            available_tilesets[0].clone()
        };
        Self {
            focused: Row::Master,
            back_focused: false,
            focus_changed: false,
            confirm_requested: false,
            cancel_requested: false,
            export_requested: false,
            profile_select_requested: false,
            scroll: SmoothScroll::new(),
            dragging_slider: None,
            master_volume: settings.master_volume,
            sfx_volume: settings.sfx_volume,
            music_volume: settings.music_volume,
            sfx_enabled: settings.sfx_enabled,
            effects_quality: settings.effects_quality,
            tile_preset: settings.tile_preset,
            tileset_name,
            available_tilesets,
            surface_kind: settings.surface_kind,
            gamma: settings.gamma,
            shadows_enabled: settings.shadows_enabled,
            ssr_enabled: settings.ssr_enabled,
            hdr_enabled: settings.hdr_enabled,
            borderless_fullscreen: settings.borderless_fullscreen,
            swap_ab: settings.swap_ab,
            swap_xy: settings.swap_xy,
            controller_layout_user_set: settings.controller_layout_user_set,
            xy_quick_action: settings.xy_quick_action,
            hold_to_sell_rumble: settings.hold_to_sell_rumble,
            auto_cash_in_on_full_structure: settings.auto_cash_in_on_full_structure,
            hints_enabled: settings.hints_enabled,
            discard_undo_enabled: settings.discard_undo_enabled,
            glyph_prompt: settings.glyph_prompt,
        }
    }

    fn cycle_tileset(&mut self, delta: isize) {
        if self.available_tilesets.is_empty() {
            return;
        }
        let len = self.available_tilesets.len() as isize;
        let cur = self
            .available_tilesets
            .iter()
            .position(|n| n == &self.tileset_name)
            .unwrap_or(0) as isize;
        let next = ((cur + delta).rem_euclid(len)) as usize;
        self.tileset_name = self.available_tilesets[next].clone();
    }

    fn cycle_surface(&mut self, delta: isize) {
        self.surface_kind = if delta >= 0 {
            self.surface_kind.next()
        } else {
            self.surface_kind.prev()
        };
    }

    fn save_settings(&self) {
        let mut settings = crate::persistence::load_settings();
        settings.master_volume = self.master_volume;
        settings.sfx_volume = self.sfx_volume;
        settings.music_volume = self.music_volume;
        settings.sfx_enabled = self.sfx_enabled;
        settings.effects_quality = self.effects_quality;
        settings.tile_preset = self.tile_preset;
        settings.tileset_name = self.tileset_name.clone();
        settings.surface_kind = self.surface_kind;
        settings.gamma = self.gamma;
        settings.shadows_enabled = self.shadows_enabled;
        settings.ssr_enabled = self.ssr_enabled;
        settings.hdr_enabled = self.hdr_enabled;
        settings.borderless_fullscreen = self.borderless_fullscreen;
        settings.swap_ab = self.swap_ab;
        settings.swap_xy = self.swap_xy;
        settings.controller_layout_user_set = self.controller_layout_user_set;
        settings.xy_quick_action = self.xy_quick_action;
        settings.hold_to_sell_rumble = self.hold_to_sell_rumble;
        settings.auto_cash_in_on_full_structure = self.auto_cash_in_on_full_structure;
        settings.hints_enabled = self.hints_enabled;
        settings.discard_undo_enabled = self.discard_undo_enabled;
        settings.glyph_prompt = self.glyph_prompt;
        let _ = crate::persistence::save_settings(&settings);
    }

    pub fn take_focus_changed(&mut self) -> bool {
        let changed = self.focus_changed;
        self.focus_changed = false;
        changed
    }

    pub fn take_confirm_requested(&mut self) -> bool {
        let requested = self.confirm_requested;
        self.confirm_requested = false;
        requested
    }

    pub fn take_cancel_requested(&mut self) -> bool {
        let requested = self.cancel_requested;
        self.cancel_requested = false;
        requested
    }

    pub fn take_export_requested(&mut self) -> bool {
        let v = self.export_requested;
        self.export_requested = false;
        v
    }

    pub fn take_profile_select_requested(&mut self) -> bool {
        let v = self.profile_select_requested;
        self.profile_select_requested = false;
        v
    }

    /// Range (min, max, step) for a slider row.
    fn slider_range(row: Row) -> (f32, f32, f32) {
        match row {
            Row::Gamma => (
                crate::persistence::GAMMA_MIN,
                crate::persistence::GAMMA_MAX,
                GAMMA_STEP,
            ),
            _ => (0.0, 1.0, VOL_STEP),
        }
    }

    fn slider_value(&self, row: Row) -> Option<f32> {
        Some(match row {
            Row::Master => self.master_volume,
            Row::Music => self.music_volume,
            Row::Sfx => self.sfx_volume,
            Row::Gamma => self.gamma,
            _ => return None,
        })
    }

    fn store_slider(&mut self, row: Row, value: f32) {
        let (lo, hi, step) = Self::slider_range(row);
        let snapped = ((value - lo) / step).round() * step + lo;
        let clamped = snapped.clamp(lo, hi);
        match row {
            Row::Master => self.master_volume = clamped,
            Row::Music => self.music_volume = clamped,
            Row::Sfx => self.sfx_volume = clamped,
            Row::Gamma => self.gamma = clamped,
            _ => {}
        }
    }

    fn adjust_slider(&mut self, row: Row, delta_steps: f32) {
        let (_, _, step) = Self::slider_range(row);
        if let Some(cur) = self.slider_value(row) {
            self.store_slider(row, cur + delta_steps * step);
            self.save_settings();
        }
    }

    fn slider_track_xw(layout: &PanelLayout) -> (f32, f32) {
        let cols = row_columns(layout.content_x, layout.content_w);
        let slider_w = cols.control_w - cols.readout_w;
        (cols.control_x, slider_w.max(1.0))
    }

    fn cursor_on_slider_track(layout: &PanelLayout, cursor_pos: (f32, f32)) -> bool {
        let (slider_x, slider_w) = Self::slider_track_xw(layout);
        let cx = cursor_pos.0;
        cx >= slider_x && cx <= slider_x + slider_w
    }

    fn set_slider_from_cursor(&mut self, row: Row, layout: &PanelLayout, cursor_pos: (f32, f32)) {
        let (slider_x, slider_w) = Self::slider_track_xw(layout);
        let cx = cursor_pos.0;
        let ratio = ((cx - slider_x) / slider_w).clamp(0.0, 1.0);
        let (lo, hi, _) = Self::slider_range(row);
        self.store_slider(row, lo + ratio * (hi - lo));
    }

    /// Clamp scroll offset and update max for the given layout.
    fn sync_scroll(&self, layout: &PanelLayout) {
        let max = CONTENT.len().saturating_sub(layout.visible_slots) as u32;
        self.scroll.set_max(max);
    }

    /// Adjust scroll so `self.focused` is visible.
    fn ensure_focused_visible(&self, layout: &PanelLayout) {
        let idx = content_index_of_row(self.focused) as f32;
        let scroll = self.scroll.target();
        let vis = layout.visible_slots as f32;
        if idx < scroll {
            self.scroll.set_target(idx);
        } else if idx >= scroll + vis {
            self.scroll.set_target(idx - vis + 1.0);
        }
    }

    /// Adjust the focused row's value rightward (increase slider, next cycle).
    fn adjust_row_right(&mut self) {
        let focused = self.focused;
        if focused.is_slider() {
            self.adjust_slider(focused, 1.0);
            return;
        }
        match focused {
            Row::SfxToggle => self.sfx_enabled = !self.sfx_enabled,
            Row::Effects => self.effects_quality = self.effects_quality.next(),
            Row::Tile => self.tile_preset = self.tile_preset.next(),
            Row::Tileset => self.cycle_tileset(1),
            Row::Surface => self.cycle_surface(1),
            Row::BorderlessFullscreen => self.borderless_fullscreen = !self.borderless_fullscreen,
            Row::Shadows => self.shadows_enabled = !self.shadows_enabled,
            Row::Ssr => self.ssr_enabled = !self.ssr_enabled,
            Row::Hdr => self.hdr_enabled = !self.hdr_enabled,
            Row::SwapAb => {
                self.swap_ab = !self.swap_ab;
                self.controller_layout_user_set = true;
            }
            Row::SwapXy => {
                self.swap_xy = !self.swap_xy;
                self.controller_layout_user_set = true;
            }
            Row::XyQuickAction => self.xy_quick_action = !self.xy_quick_action,
            Row::HoldToSellRumble => self.hold_to_sell_rumble = !self.hold_to_sell_rumble,
            Row::AutoCashInOnFullStructure => {
                self.auto_cash_in_on_full_structure = !self.auto_cash_in_on_full_structure
            }
            Row::Hints => self.hints_enabled = !self.hints_enabled,
            Row::GlyphPrompts => self.glyph_prompt = self.glyph_prompt.next(),
            Row::UndoDiscard => self.discard_undo_enabled = !self.discard_undo_enabled,
            Row::ExportPlayStats | Row::SaveAndProfiles => return,
            _ => return,
        }
        self.save_settings();
    }

    /// Adjust the focused row's value leftward (decrease slider, prev cycle).
    fn adjust_row_left(&mut self) {
        let focused = self.focused;
        if focused.is_slider() {
            self.adjust_slider(focused, -1.0);
            return;
        }
        match focused {
            Row::SfxToggle => self.sfx_enabled = !self.sfx_enabled,
            Row::Effects => self.effects_quality = self.effects_quality.prev(),
            Row::Tile => self.tile_preset = self.tile_preset.prev(),
            Row::Tileset => self.cycle_tileset(-1),
            Row::Surface => self.cycle_surface(-1),
            Row::BorderlessFullscreen => self.borderless_fullscreen = !self.borderless_fullscreen,
            Row::Shadows => self.shadows_enabled = !self.shadows_enabled,
            Row::Ssr => self.ssr_enabled = !self.ssr_enabled,
            Row::Hdr => self.hdr_enabled = !self.hdr_enabled,
            Row::SwapAb => {
                self.swap_ab = !self.swap_ab;
                self.controller_layout_user_set = true;
            }
            Row::SwapXy => {
                self.swap_xy = !self.swap_xy;
                self.controller_layout_user_set = true;
            }
            Row::XyQuickAction => self.xy_quick_action = !self.xy_quick_action,
            Row::HoldToSellRumble => self.hold_to_sell_rumble = !self.hold_to_sell_rumble,
            Row::AutoCashInOnFullStructure => {
                self.auto_cash_in_on_full_structure = !self.auto_cash_in_on_full_structure
            }
            Row::Hints => self.hints_enabled = !self.hints_enabled,
            Row::GlyphPrompts => self.glyph_prompt = self.glyph_prompt.prev(),
            Row::UndoDiscard => self.discard_undo_enabled = !self.discard_undo_enabled,
            Row::ExportPlayStats | Row::SaveAndProfiles => return,
            _ => return,
        }
        self.save_settings();
    }

    /// Apply a click/confirm on a row. Returns true if the scene should close.
    fn apply_click(&mut self, row: Row, layout: &PanelLayout, cursor_pos: (f32, f32)) -> bool {
        match row {
            Row::Master | Row::Music | Row::Sfx | Row::Gamma => {
                if Self::cursor_on_slider_track(layout, cursor_pos) {
                    self.set_slider_from_cursor(row, layout, cursor_pos);
                    self.save_settings();
                }
            }
            Row::SfxToggle => {
                self.sfx_enabled = !self.sfx_enabled;
                self.save_settings();
            }
            Row::Effects => {
                self.effects_quality = self.effects_quality.next();
                self.save_settings();
            }
            Row::Tile => {
                self.tile_preset = self.tile_preset.next();
                self.save_settings();
            }
            Row::Tileset => {
                self.cycle_tileset(1);
                self.save_settings();
            }
            Row::Surface => {
                self.cycle_surface(1);
                self.save_settings();
            }
            Row::BorderlessFullscreen => {
                self.borderless_fullscreen = !self.borderless_fullscreen;
                self.save_settings();
            }
            Row::Shadows => {
                self.shadows_enabled = !self.shadows_enabled;
                self.save_settings();
            }
            Row::Ssr => {
                self.ssr_enabled = !self.ssr_enabled;
                self.save_settings();
            }
            Row::Hdr => {
                self.hdr_enabled = !self.hdr_enabled;
                self.save_settings();
            }
            Row::SwapAb => {
                self.swap_ab = !self.swap_ab;
                self.controller_layout_user_set = true;
                self.save_settings();
            }
            Row::SwapXy => {
                self.swap_xy = !self.swap_xy;
                self.controller_layout_user_set = true;
                self.save_settings();
            }
            Row::XyQuickAction => {
                self.xy_quick_action = !self.xy_quick_action;
                self.save_settings();
            }
            Row::HoldToSellRumble => {
                self.hold_to_sell_rumble = !self.hold_to_sell_rumble;
                self.save_settings();
            }
            Row::AutoCashInOnFullStructure => {
                self.auto_cash_in_on_full_structure = !self.auto_cash_in_on_full_structure;
                self.save_settings();
            }
            Row::Hints => {
                self.hints_enabled = !self.hints_enabled;
                self.save_settings();
            }
            Row::GlyphPrompts => {
                self.glyph_prompt = self.glyph_prompt.next();
                self.save_settings();
            }
            Row::UndoDiscard => {
                self.discard_undo_enabled = !self.discard_undo_enabled;
                self.save_settings();
            }
            Row::ExportPlayStats => {
                self.export_requested = true;
            }
            Row::SaveAndProfiles => {
                self.profile_select_requested = true;
            }
        }
        false
    }

    // ── Public interface (shared with pause-menu overlay) ──────────────

    /// Process one frame of input. Returns `true` if the user requested to
    /// close (Back / Cancel / Pause).
    pub fn update_input(&mut self, input: OptionsInput<'_>) -> bool {
        let OptionsInput {
            actions,
            button_clicks,
            cursor_pos,
            window_w,
            window_h,
            scroll_lines,
            input_mode,
            mouse_left_down,
        } = input;
        self.focus_changed = false;
        self.confirm_requested = false;
        self.cancel_requested = false;
        if !mouse_left_down {
            self.dragging_slider = None;
        }
        let layout = compute_layout(window_w, window_h);
        self.sync_scroll(&layout);
        let prev_focus = (self.focused, self.back_focused);

        // ── Scroll wheel ───────────────────────────────────────────────
        // Apply when the cursor is over the content area.
        if scroll_lines.abs() > 0.001 {
            let (cx, cy) = cursor_pos;
            let content_end_y = layout.content_start_y
                + layout.visible_slots as f32 * (layout.slot_h + layout.slot_gap);
            if cx >= layout.content_x
                && cx <= layout.content_x + layout.content_w
                && cy >= layout.content_start_y
                && cy <= content_end_y
            {
                // Pass the raw float so trackpad momentum isn't rounded away.
                self.scroll.scroll_by(-scroll_lines);
            }
        }

        // ── Mouse hover ────────────────────────────────────────────────
        // Match widget-tree / main-menu: only the mouse drives hover in cursor
        // mode so controller / keyboard focus is not overwritten by a parked cursor.
        if input_mode == InputMode::Cursor {
            let (cx, cy) = cursor_pos;
            let scroll = self.scroll.target() as usize;
            for (vi, ci) in (scroll..CONTENT.len()).enumerate() {
                if vi >= layout.visible_slots {
                    break;
                }
                if let ContentSlot::Row(row) = CONTENT[ci] {
                    let ry = layout.content_start_y + vi as f32 * (layout.slot_h + layout.slot_gap);
                    if cx >= layout.content_x
                        && cx <= layout.content_x + layout.content_w
                        && cy >= ry
                        && cy <= ry + layout.slot_h
                    {
                        self.focused = row;
                        self.back_focused = false;
                    }
                }
            }
            if cx >= layout.back_x
                && cx <= layout.back_x + layout.back_w
                && cy >= layout.back_y
                && cy <= layout.back_y + layout.back_h
            {
                self.back_focused = true;
            }
        }

        // ── Button clicks ──────────────────────────────────────────────
        for &cid in button_clicks {
            // TOC link?
            if cid >= TOC_ID_BASE && cid < TOC_ID_BASE + SECTIONS.len() as u32 {
                let section = SECTIONS[(cid - TOC_ID_BASE) as usize];
                let target = content_index_of_section(section) as f32;
                self.scroll.set_target(target);
                if let Some(first_row) = CONTENT[target as usize..].iter().find_map(|s| match s {
                    ContentSlot::Row(r) => Some(*r),
                    _ => None,
                }) {
                    self.focused = first_row;
                    self.back_focused = false;
                }
                continue;
            }
            if cid == BACK_ID {
                self.save_settings();
                self.cancel_requested = true;
                return true;
            }
            if let Some(row) = Row::from_click_id(cid) {
                self.focused = row;
                self.back_focused = false;
                self.confirm_requested = true;
                let close = self.apply_click(row, &layout, cursor_pos);
                if row.is_slider() && Self::cursor_on_slider_track(&layout, cursor_pos) {
                    self.dragging_slider = Some(row);
                }
                if close {
                    return true;
                }
                return false;
            }
        }

        if mouse_left_down && let Some(row) = self.dragging_slider {
            self.set_slider_from_cursor(row, &layout, cursor_pos);
            self.save_settings();
        }

        // ── Keyboard / gamepad ─────────────────────────────────────────
        for a in actions {
            match a {
                UiAction::FocusDown if self.back_focused => {}
                UiAction::FocusUp if self.back_focused => {
                    self.back_focused = false;
                    self.focused = *ROWS.last().unwrap();
                    self.ensure_focused_visible(&layout);
                }
                UiAction::FocusDown => {
                    let idx = ROWS.iter().position(|&r| r == self.focused).unwrap_or(0);
                    if idx + 1 < ROWS.len() {
                        self.focused = ROWS[idx + 1];
                        self.ensure_focused_visible(&layout);
                    } else {
                        self.back_focused = true;
                    }
                }
                UiAction::FocusUp => {
                    let idx = ROWS.iter().position(|&r| r == self.focused).unwrap_or(0);
                    if idx > 0 {
                        self.focused = ROWS[idx - 1];
                        self.ensure_focused_visible(&layout);
                    }
                }
                UiAction::FocusNext => {
                    if !self.back_focused {
                        self.adjust_row_right();
                    }
                }
                UiAction::FocusPrev => {
                    if !self.back_focused {
                        self.adjust_row_left();
                    }
                }
                UiAction::Confirm | UiAction::CommitDiscard if self.back_focused => {
                    self.save_settings();
                    self.cancel_requested = true;
                    return true;
                }
                UiAction::Confirm | UiAction::CommitDiscard => {
                    self.confirm_requested = true;
                    if self.apply_click(self.focused, &layout, cursor_pos) {
                        return true;
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    self.save_settings();
                    self.cancel_requested = true;
                    return true;
                }
                _ => {}
            }
        }
        self.focus_changed = prev_focus != (self.focused, self.back_focused);
        false
    }

    fn hint_text(&self, back_focused: bool, hint: &OptionsDrawHint) -> String {
        if matches!(hint.input_mode, InputMode::Controller) {
            let confirm = ButtonPrompt::controller_action_label(
                hint.gamepad_style,
                UiAction::Confirm,
                hint.swap_ab,
                hint.swap_xy,
            )
            .unwrap_or("A");
            let cancel = ButtonPrompt::controller_action_label(
                hint.gamepad_style,
                UiAction::Cancel,
                hint.swap_ab,
                hint.swap_xy,
            )
            .unwrap_or("B");
            if back_focused {
                return format!("{confirm}: back   ·   {cancel}: back");
            }
            if self.focused.is_slider() {
                return format!("←→: adjust   ·   ↑↓: navigate   ·   {cancel}: back");
            }
            if matches!(self.focused, Row::ExportPlayStats | Row::SaveAndProfiles) {
                return format!("{confirm}: select   ·   ↑↓: navigate   ·   {cancel}: back");
            }
            return format!(
                "←→: change   ·   {confirm}: toggle   ·   ↑↓: navigate   ·   {cancel}: back"
            );
        }

        if back_focused {
            "Enter: back · Esc: back".into()
        } else if self.focused.is_slider() {
            "←→ or drag: adjust · ↑↓: navigate · Esc: back".into()
        } else if matches!(self.focused, Row::ExportPlayStats | Row::SaveAndProfiles) {
            "Enter: select · ↑↓: navigate · Esc: back".into()
        } else {
            "←→: change · Enter: toggle · ↑↓: navigate · Esc: back".into()
        }
    }

    /// Build the options menu UI elements into the supplied buffers.
    pub fn draw_overlay(
        &self,
        w: f32,
        h: f32,
        hint: OptionsDrawHint,
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        buttons: &mut Vec<ButtonDef>,
    ) {
        let layout = compute_layout(w, h);
        self.sync_scroll(&layout);
        let smooth = self.scroll.tick();
        let cols = row_columns(layout.content_x, layout.content_w);

        // ── Title ──────────────────────────────────────────────────────
        let title_font = typography::size(typography::H16, h);
        text_labels.push(TextLabel {
            rect: [0.0, layout.title_y, w, layout.title_h],
            text: "Options".into(),
            color: color::CHAMPAGNE,
            font_px: Some(title_font),
            ..Default::default()
        });
        let rule_y = layout.title_y + layout.title_h + (4.0 * layout.scale);
        instances.push(GpuInstance {
            rect: [
                w * 0.25,
                rule_y,
                w * 0.5,
                (1.0 * layout.scale).max(1.0),
            ],
            color: color::BRASS,
            user: 0,
        });

        // ── TOC column ─────────────────────────────────────────────────
        let active_sec = section_at_scroll(smooth);
        for (i, &section) in SECTIONS.iter().enumerate() {
            let y = layout.toc_start_y + i as f32 * (layout.toc_item_h + layout.toc_gap);
            let is_active = section == active_sec;

            if is_active {
                instances.push(GpuInstance {
                    rect: [layout.toc_x, y, layout.toc_w, layout.toc_item_h],
                    color: color::WALNUT_BRIGHT,
                    user: 0,
                });
            }

            let text_color = if is_active {
                color::CHAMPAGNE
            } else {
                color::UMBER
            };
            text_labels.push(TextLabel {
                rect: [
                    layout.toc_x + 8.0 * layout.scale,
                    y,
                    layout.toc_w - 8.0 * layout.scale,
                    layout.toc_item_h,
                ],
                text: section.label().into(),
                color: text_color,
                align: TextAlign::Left,
                ..Default::default()
            });
            buttons.push(ButtonDef::scene(
                (layout.toc_x, y, layout.toc_w, layout.toc_item_h),
                TOC_ID_BASE + i as u32,
            ));
        }

        // ── Scrollable content ─────────────────────────────────────────
        let scroll = smooth.floor() as usize;
        let frac_offset = -(smooth.fract()) * (layout.slot_h + layout.slot_gap);
        let track_h = (8.0 * layout.scale).max(4.0);
        let content_clip_rect = [
            layout.content_x,
            layout.content_start_y,
            layout.content_w,
            layout.visible_slots as f32 * (layout.slot_h + layout.slot_gap),
        ];

        let header_font = typography::size(typography::H28, h);
        let render_slots = layout.visible_slots + 1;
        for (vi, ci) in (scroll..CONTENT.len()).enumerate() {
            if vi >= render_slots {
                break;
            }
            let slot_y = layout.content_start_y
                + vi as f32 * (layout.slot_h + layout.slot_gap)
                + frac_offset;
            let slot_rect = [layout.content_x, slot_y, layout.content_w, layout.slot_h];
            let Some(clipped_slot_rect) = intersect_rect(slot_rect, content_clip_rect) else {
                continue;
            };
            match CONTENT[ci] {
                ContentSlot::Header(section) => {
                    text_labels.push(TextLabel {
                        rect: slot_rect,
                        text: section.label().into(),
                        color: color::GOLD,
                        font_px: Some(header_font),
                        align: TextAlign::Left,
                        clip_rect: Some(content_clip_rect),
                        ..Default::default()
                    });
                    let line_y = slot_y + layout.slot_h - (1.0 * layout.scale);
                    if let Some(line_rect) = intersect_rect(
                        [
                            layout.content_x,
                            line_y,
                            layout.content_w,
                            (1.0 * layout.scale).max(1.0),
                        ],
                        content_clip_rect,
                    ) {
                        instances.push(GpuInstance {
                            rect: line_rect,
                            color: color::ANTIQUE,
                            user: 0,
                        });
                    }
                }
                ContentSlot::Row(row) => {
                    let is_focused = !self.back_focused && row == self.focused;
                    self.draw_row(
                        instances,
                        text_labels,
                        row,
                        slot_rect,
                        content_clip_rect,
                        is_focused,
                        track_h,
                        &cols,
                    );
                    buttons.push(ButtonDef::scene(
                        (
                            clipped_slot_rect[0],
                            clipped_slot_rect[1],
                            clipped_slot_rect[2],
                            clipped_slot_rect[3],
                        ),
                        row.click_id(),
                    ));
                }
            }
        }

        // ── Scroll edge fades ──────────────────────────────────────────
        let max_scroll = self.scroll.max();
        let fade_h = (16.0 * layout.scale).max(10.0);
        let content_h = content_clip_rect[3];
        if smooth > 0.01 {
            instances.push(GpuInstance {
                rect: [
                    layout.content_x,
                    layout.content_start_y,
                    layout.content_w,
                    fade_h.min(content_h * 0.35),
                ],
                color: color::alpha(hint.scroll_fade_backdrop, 0.72),
                user: 0,
            });
        }
        if max_scroll > 0.0 && smooth < max_scroll - 0.01 {
            instances.push(GpuInstance {
                rect: [
                    layout.content_x,
                    layout.content_start_y + content_h - fade_h.min(content_h * 0.35),
                    layout.content_w,
                    fade_h.min(content_h * 0.35),
                ],
                color: color::alpha(hint.scroll_fade_backdrop, 0.72),
                user: 0,
            });
        }

        // ── Scroll indicator ───────────────────────────────────────────
        if max_scroll > 0.0 {
            let indicator_x = layout.content_x + layout.content_w + (6.0 * layout.scale);
            let indicator_y = layout.content_start_y;
            let indicator_w = (7.0 * layout.scale).max(6.0);
            let indicator_h = layout.visible_slots as f32 * (layout.slot_h + layout.slot_gap);
            instances.push(GpuInstance {
                rect: [indicator_x, indicator_y, indicator_w, indicator_h],
                color: color::WALNUT_RAISED,
                user: 0,
            });
            let thumb_h = (indicator_h * (layout.visible_slots as f32 / CONTENT.len() as f32))
                .max(12.0 * layout.scale);
            let thumb_y = indicator_y + (indicator_h - thumb_h) * (smooth / max_scroll);
            instances.push(GpuInstance {
                rect: [indicator_x, thumb_y, indicator_w, thumb_h],
                color: color::WALNUT_BRIGHT,
                user: 0,
            });
        }

        // ── Back button ────────────────────────────────────────────────
        let back_bg = if self.back_focused {
            color::WALNUT_BRIGHT
        } else {
            color::WALNUT_RAISED
        };
        instances.push(GpuInstance {
            rect: [layout.back_x, layout.back_y, layout.back_w, layout.back_h],
            color: back_bg,
            user: 0,
        });
        let back_text = if self.back_focused {
            color::CHAMPAGNE
        } else {
            color::STONE
        };
        text_labels.push(TextLabel {
            rect: [layout.back_x, layout.back_y, layout.back_w, layout.back_h],
            text: "Back".into(),
            color: back_text,
            ..Default::default()
        });
        buttons.push(ButtonDef::scene(
            (layout.back_x, layout.back_y, layout.back_w, layout.back_h),
            BACK_ID,
        ));

        // ── Hint ───────────────────────────────────────────────────────
        text_labels.push(TextLabel {
            rect: [0.0, layout.hint_y, w, layout.hint_h],
            text: self.hint_text(self.back_focused, &hint),
            color: color::STONE,
            ..Default::default()
        });

        let version_text = if cfg!(debug_assertions) {
            "vNEXT".into()
        } else {
            format!("v{}", env!("CARGO_PKG_VERSION"))
        };
        let version_font = typography::size(typography::H45, h);
        text_labels.push(TextLabel {
            rect: [0.0, layout.version_y, w, layout.version_h],
            text: version_text,
            font_px: Some(version_font),
            color: color::STONE,
            ..Default::default()
        });
    }

    // ── Row rendering ──────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        row: Row,
        rect: [f32; 4],
        content_clip_rect: [f32; 4],
        is_focused: bool,
        track_h: f32,
        cols: &RowColumns,
    ) {
        let [row_x, row_y, row_w, row_h] = rect;
        let scale = (row_h / 40.0).max(0.5);
        let accent_w = (3.0 * scale).max(2.0);
        let pad = 8.0 * scale;
        let mut push_quad_in_clip = |quad_rect: [f32; 4], quad_color: [f32; 4]| {
            if let Some(clipped) = intersect_rect(quad_rect, content_clip_rect) {
                instances.push(GpuInstance {
                    rect: clipped,
                    color: quad_color,
                    user: 0,
                });
            }
        };

        let label_color = if is_focused {
            color::CHAMPAGNE
        } else {
            color::STONE
        };
        let value_color = if is_focused {
            color::PARCHMENT
        } else {
            color::STONE
        };

        match row {
            Row::Master | Row::Music | Row::Sfx | Row::Gamma => {
                let value = self.slider_value(row).unwrap_or(0.0);
                let (lo, hi, _) = Self::slider_range(row);
                let fill_ratio = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
                let (label, readout) = row_copy(row, self);

                let bg_color = if is_focused {
                    color::WALNUT_SOFT
                } else {
                    color::WALNUT_RAISED
                };
                push_quad_in_clip([row_x, row_y, row_w, row_h], bg_color);
                if is_focused {
                    push_quad_in_clip([row_x, row_y, accent_w, row_h], color::GOLD);
                }

                text_labels.push(TextLabel {
                    rect: [
                        cols.label_x + pad,
                        row_y,
                        cols.label_w - pad * 2.0,
                        row_h,
                    ],
                    text: label.into(),
                    color: label_color,
                    align: TextAlign::Left,
                    clip_rect: Some(content_clip_rect),
                    ..Default::default()
                });

                let slider_w = cols.control_w - cols.readout_w;
                let slider_x = cols.control_x;
                let track_y = row_y + (row_h - track_h) * 0.5;
                push_quad_in_clip([slider_x, track_y, slider_w, track_h], color::WALNUT_INK);
                let fill_w = slider_w * fill_ratio;
                let fill_color = if is_focused {
                    color::GOLD
                } else {
                    color::BRASS
                };
                push_quad_in_clip([slider_x, track_y, fill_w, track_h], fill_color);
                let knob_size = track_h * 2.5;
                let knob_x = slider_x + fill_w - knob_size * 0.5;
                let knob_y = track_y + (track_h - knob_size) * 0.5;
                let knob_color = if is_focused {
                    color::CHAMPAGNE
                } else {
                    color::PARCHMENT
                };
                push_quad_in_clip([knob_x, knob_y, knob_size, knob_size], knob_color);
                text_labels.push(TextLabel {
                    rect: [cols.readout_x, row_y, cols.readout_w - pad, row_h],
                    text: readout,
                    color: value_color,
                    align: TextAlign::Right,
                    clip_rect: Some(content_clip_rect),
                    ..Default::default()
                });
            }
            _ => {
                let (label, value) = row_copy(row, self);
                let bg_color = if is_focused {
                    color::WALNUT_SOFT
                } else {
                    color::WALNUT_RAISED
                };
                push_quad_in_clip([row_x, row_y, row_w, row_h], bg_color);
                if is_focused {
                    push_quad_in_clip([row_x, row_y, accent_w, row_h], color::GOLD);
                }

                text_labels.push(TextLabel {
                    rect: [
                        cols.label_x + pad,
                        row_y,
                        cols.label_w - pad * 2.0,
                        row_h,
                    ],
                    text: label.into(),
                    color: label_color,
                    align: TextAlign::Left,
                    clip_rect: Some(content_clip_rect),
                    ..Default::default()
                });

                if !value.is_empty() {
                    text_labels.push(TextLabel {
                        rect: [
                            cols.control_x,
                            row_y,
                            cols.control_w + cols.readout_w - pad,
                            row_h,
                        ],
                        text: value,
                        color: value_color,
                        align: TextAlign::Right,
                        clip_rect: Some(content_clip_rect),
                        ..Default::default()
                    });
                }
            }
        }
    }
}

// ── SceneBehavior (standalone options screen) ──────────────────────────

impl SceneBehavior for OptionsScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if self.update_input(OptionsInput {
            actions: ctx.actions,
            button_clicks: ctx.button_clicks,
            cursor_pos: ctx.cursor_pos,
            window_w: ctx.layout.window_w,
            window_h: ctx.layout.window_h,
            scroll_lines: ctx.scroll_lines,
            input_mode: ctx.input_mode,
            mouse_left_down: ctx.mouse_left_down,
        }) {
            if self.take_cancel_requested() {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
            }
            return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
        }
        if self.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        if self.take_confirm_requested() {
            ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
        }
        if self.take_export_requested() {
            let path = crate::persistence::play_stats_export_path(ctx.active_profile);
            match crate::bot::export_play_history_html(&path, ctx.progress) {
                Ok(()) => ctx.bus.push(GameEvent::InfoModal {
                    title: "Stats exported".into(),
                    body: format!("Saved HTML report to:\n{}", path.display()),
                }),
                Err(e) => ctx.bus.push(GameEvent::InfoModal {
                    title: "Export failed".into(),
                    body: format!("{e:#}"),
                }),
            }
        }
        if self.take_profile_select_requested() {
            return Some(Scene::ProfileSelect(ProfileSelectScene::from_options_menu()));
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::WALNUT_INK,
            user: 0,
        }];
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();

        self.draw_overlay(
            w,
            h,
            OptionsDrawHint::from_draw_ctx(&ctx),
            &mut instances,
            &mut text_labels,
            &mut buttons,
        );

        let mut frame = UiFrame::new();
        frame.quads(instances);
        frame.texts(text_labels);
        frame.buttons = buttons;
        frame.window_title = "Mahjuro — Options".into();
        frame
    }
}
