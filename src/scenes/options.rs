//! Options scene — volume sliders, visual settings, and rendering options.
//!
//! Layout: a table-of-contents (TOC) column on the left links to three
//! sections (Audio, Visual, Rendering) in a scrollable content pane on
//! the right.  Entry-based scroll stepping (same pattern as the glossary)
//! keeps every visible row fully on-screen — the renderer has no scissor
//! support.

use crate::audio::SfxId;
use crate::game::event_bus::GameEvent;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::smooth_scroll::SmoothScroll;

use crate::render::draw_cmd::UiFrame;

use super::start_screen::StartScreenScene;
use super::{ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

// ── Constants ──────────────────────────────────────────────────────────

/// Volume adjustment step per input press.
const VOL_STEP: f32 = 0.05;
/// Gamma adjustment step per input press.
const GAMMA_STEP: f32 = 0.05;
/// UI scale adjustment step per input press.
const UI_SCALE_STEP: f32 = 0.05;

/// Click-id base for TOC links (high range to avoid collisions).
const TOC_ID_BASE: u32 = 0xF200;
/// Click-id for the fixed Back button.
const BACK_ID: u32 = 0xF210;

// ── Sections ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Section {
    Audio,
    Visual,
    Rendering,
    Controls,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Section::Audio => "Audio",
            Section::Visual => "Visual",
            Section::Rendering => "Rendering",
            Section::Controls => "Controls",
        }
    }
}

const SECTIONS: &[Section] = &[
    Section::Audio,
    Section::Visual,
    Section::Rendering,
    Section::Controls,
];

// ── Rows ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Row {
    Master,
    Music,
    Sfx,
    SfxToggle,
    Gamma,
    SmokeQuality,
    SmokeAmount,
    Effects,
    Tile,
    Tileset,
    UiScale,
    Shadows,
    Ssr,
    Hdr,
    SwapAb,
    AutoCashInOnFullStructure,
    Hints,
}

impl Row {
    fn click_id(self) -> u32 {
        self as u32 + 1
    }

    fn from_click_id(id: u32) -> Option<Row> {
        ROWS.iter().copied().find(|r| r.click_id() == id)
    }

    fn is_slider(self) -> bool {
        matches!(
            self,
            Row::Master | Row::Music | Row::Sfx | Row::Gamma | Row::UiScale
        )
    }
}

/// Navigable rows in section order (keyboard Up/Down cycles through these).
const ROWS: &[Row] = &[
    Row::Master,
    Row::Music,
    Row::Sfx,
    Row::SfxToggle,
    Row::Gamma,
    Row::SmokeQuality,
    Row::SmokeAmount,
    Row::Effects,
    Row::Tile,
    Row::Tileset,
    Row::UiScale,
    Row::Shadows,
    Row::Ssr,
    Row::Hdr,
    Row::SwapAb,
    Row::AutoCashInOnFullStructure,
    Row::Hints,
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
    ContentSlot::Header(Section::Visual),
    ContentSlot::Row(Row::Gamma),
    ContentSlot::Row(Row::SmokeQuality),
    ContentSlot::Row(Row::SmokeAmount),
    ContentSlot::Row(Row::Effects),
    ContentSlot::Row(Row::Tile),
    ContentSlot::Row(Row::Tileset),
    ContentSlot::Row(Row::UiScale),
    ContentSlot::Header(Section::Rendering),
    ContentSlot::Row(Row::Shadows),
    ContentSlot::Row(Row::Ssr),
    ContentSlot::Row(Row::Hdr),
    ContentSlot::Header(Section::Controls),
    ContentSlot::Row(Row::SwapAb),
    ContentSlot::Row(Row::AutoCashInOnFullStructure),
    ContentSlot::Row(Row::Hints),
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

/// Which section does the given row belong to? Walks backwards through
/// CONTENT from the row's position to find its enclosing Header.
fn section_of_row(row: Row) -> Section {
    let idx = content_index_of_row(row);
    for slot in CONTENT[..idx].iter().rev() {
        if let ContentSlot::Header(sec) = slot {
            return *sec;
        }
    }
    Section::Audio
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
}

fn compute_layout(w: f32, h: f32, ui_scale: f32) -> PanelLayout {
    let scale = (w.min(h)) / 600.0 * ui_scale;

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

    // Back button and hint pinned to the bottom.
    let back_h = (42.0 * scale).max(28.0);
    let hint_h = (20.0 * scale).max(14.0);
    let hint_y = h - hint_h - (8.0 * scale);
    let back_y = hint_y - back_h - (12.0 * scale);
    let back_w = (180.0 * scale).min(content_w * 0.5);
    let back_x = content_x + (content_w - back_w) * 0.5;

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
    }
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
    /// Smooth-scrolling state for the content pane.
    scroll: SmoothScroll,

    /// Local copy of settings; written back on change and scene exit.
    pub master_volume: f32,
    pub sfx_volume: f32,
    pub music_volume: f32,
    pub sfx_enabled: bool,
    pub smoke_quality: crate::persistence::SmokeQuality,
    pub smoke_amount: crate::persistence::SmokeAmount,
    pub effects_quality: crate::persistence::EffectsQuality,
    pub tile_preset: crate::persistence::TilePreset,
    pub tileset_name: String,
    pub available_tilesets: Vec<String>,
    pub gamma: f32,
    pub shadows_enabled: bool,
    pub ssr_enabled: bool,
    pub hdr_enabled: bool,
    pub swap_ab: bool,
    pub auto_cash_in_on_full_structure: bool,
    pub hints_enabled: bool,
    pub ui_scale: f32,
}

impl OptionsScene {
    pub fn new() -> Self {
        let settings = crate::persistence::load_settings();
        let mut available_tilesets = crate::asset_path::list_tilesets();
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
            scroll: SmoothScroll::new(),
            master_volume: settings.master_volume,
            sfx_volume: settings.sfx_volume,
            music_volume: settings.music_volume,
            sfx_enabled: settings.sfx_enabled,
            smoke_quality: settings.smoke_quality,
            smoke_amount: settings.smoke_amount,
            effects_quality: settings.effects_quality,
            tile_preset: settings.tile_preset,
            tileset_name,
            available_tilesets,
            gamma: settings.gamma,
            shadows_enabled: settings.shadows_enabled,
            ssr_enabled: settings.ssr_enabled,
            hdr_enabled: settings.hdr_enabled,
            swap_ab: settings.swap_ab,
            auto_cash_in_on_full_structure: settings.auto_cash_in_on_full_structure,
            hints_enabled: settings.hints_enabled,
            ui_scale: settings.ui_scale,
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

    fn save_settings(&self) {
        let mut settings = crate::persistence::load_settings();
        settings.master_volume = self.master_volume;
        settings.sfx_volume = self.sfx_volume;
        settings.music_volume = self.music_volume;
        settings.sfx_enabled = self.sfx_enabled;
        settings.smoke_quality = self.smoke_quality;
        settings.smoke_amount = self.smoke_amount;
        settings.effects_quality = self.effects_quality;
        settings.tile_preset = self.tile_preset;
        settings.tileset_name = self.tileset_name.clone();
        settings.gamma = self.gamma;
        settings.shadows_enabled = self.shadows_enabled;
        settings.ssr_enabled = self.ssr_enabled;
        settings.hdr_enabled = self.hdr_enabled;
        settings.swap_ab = self.swap_ab;
        settings.auto_cash_in_on_full_structure = self.auto_cash_in_on_full_structure;
        settings.hints_enabled = self.hints_enabled;
        settings.ui_scale = self.ui_scale;
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

    /// Range (min, max, step) for a slider row.
    fn slider_range(row: Row) -> (f32, f32, f32) {
        match row {
            Row::Gamma => (
                crate::persistence::GAMMA_MIN,
                crate::persistence::GAMMA_MAX,
                GAMMA_STEP,
            ),
            Row::UiScale => (
                crate::persistence::UI_SCALE_MIN,
                crate::persistence::UI_SCALE_MAX,
                UI_SCALE_STEP,
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
            Row::UiScale => self.ui_scale,
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
            Row::UiScale => self.ui_scale = clamped,
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
            Row::SmokeQuality => self.smoke_quality = self.smoke_quality.next(),
            Row::SmokeAmount => self.smoke_amount = self.smoke_amount.next(),
            Row::Effects => self.effects_quality = self.effects_quality.next(),
            Row::Tile => self.tile_preset = self.tile_preset.next(),
            Row::Tileset => self.cycle_tileset(1),
            Row::Shadows => self.shadows_enabled = !self.shadows_enabled,
            Row::Ssr => self.ssr_enabled = !self.ssr_enabled,
            Row::Hdr => self.hdr_enabled = !self.hdr_enabled,
            Row::SwapAb => self.swap_ab = !self.swap_ab,
            Row::AutoCashInOnFullStructure => {
                self.auto_cash_in_on_full_structure = !self.auto_cash_in_on_full_structure
            }
            Row::Hints => self.hints_enabled = !self.hints_enabled,
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
            Row::SmokeQuality => self.smoke_quality = self.smoke_quality.prev(),
            Row::SmokeAmount => self.smoke_amount = self.smoke_amount.prev(),
            Row::Effects => self.effects_quality = self.effects_quality.prev(),
            Row::Tile => self.tile_preset = self.tile_preset.prev(),
            Row::Tileset => self.cycle_tileset(-1),
            Row::Shadows => self.shadows_enabled = !self.shadows_enabled,
            Row::Ssr => self.ssr_enabled = !self.ssr_enabled,
            Row::Hdr => self.hdr_enabled = !self.hdr_enabled,
            Row::SwapAb => self.swap_ab = !self.swap_ab,
            Row::AutoCashInOnFullStructure => {
                self.auto_cash_in_on_full_structure = !self.auto_cash_in_on_full_structure
            }
            Row::Hints => self.hints_enabled = !self.hints_enabled,
            _ => return,
        }
        self.save_settings();
    }

    /// Apply a click/confirm on a row. Returns true if the scene should close.
    fn apply_click(&mut self, row: Row, layout: &PanelLayout, cursor_pos: (f32, f32)) -> bool {
        match row {
            Row::Master | Row::Music | Row::Sfx | Row::Gamma | Row::UiScale => {
                // Click-to-position on the slider track.
                let cx = cursor_pos.0;
                let label_w = layout.content_w * 0.35;
                let slider_x = layout.content_x + label_w;
                let slider_w = layout.content_w * 0.50;
                if cx >= slider_x && cx <= slider_x + slider_w {
                    let ratio = (cx - slider_x) / slider_w;
                    let (lo, hi, _) = Self::slider_range(row);
                    self.store_slider(row, lo + ratio * (hi - lo));
                    self.save_settings();
                }
            }
            Row::SfxToggle => {
                self.sfx_enabled = !self.sfx_enabled;
                self.save_settings();
            }
            Row::SmokeQuality => {
                self.smoke_quality = self.smoke_quality.next();
                self.save_settings();
            }
            Row::SmokeAmount => {
                self.smoke_amount = self.smoke_amount.next();
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
        }
        false
    }

    // ── Public interface (shared with pause-menu overlay) ──────────────

    /// Process one frame of input. Returns `true` if the user requested to
    /// close (Back / Cancel / Pause).
    pub fn update_input(
        &mut self,
        actions: &[UiAction],
        button_clicks: &[u32],
        cursor_pos: (f32, f32),
        window_w: f32,
        window_h: f32,
        scroll_lines: f32,
    ) -> bool {
        self.focus_changed = false;
        self.confirm_requested = false;
        self.cancel_requested = false;
        let layout = compute_layout(window_w, window_h, self.ui_scale);
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

        // ── Button clicks ──────────────────────────────────────────────
        for &cid in button_clicks {
            // TOC link?
            if cid >= TOC_ID_BASE && cid < TOC_ID_BASE + SECTIONS.len() as u32 {
                let section = SECTIONS[(cid - TOC_ID_BASE) as usize];
                let target = content_index_of_section(section) as f32;
                self.scroll.set_target(target);
                // Focus the first row of this section.
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
                return self.apply_click(row, &layout, cursor_pos);
            }
        }

        // ── Keyboard / gamepad ─────────────────────────────────────────
        for a in actions {
            match a {
                UiAction::FocusDown if self.back_focused => {
                    // Wrap to first row.
                    self.focused = ROWS[0];
                    self.back_focused = false;
                    self.scroll.set_target(0.0);
                }
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

    /// Build the options menu UI elements into the supplied buffers.
    pub fn draw_overlay(
        &self,
        w: f32,
        h: f32,
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        buttons: &mut Vec<ButtonDef>,
    ) {
        let layout = compute_layout(w, h, self.ui_scale);
        self.sync_scroll(&layout);

        // ── Title ──────────────────────────────────────────────────────
        text_labels.push(TextLabel {
            rect: [0.0, layout.title_y, w, layout.title_h],
            text: "Options".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // ── TOC column ─────────────────────────────────────────────────
        let active_sec = section_of_row(self.focused);
        for (i, &section) in SECTIONS.iter().enumerate() {
            let y = layout.toc_start_y + i as f32 * (layout.toc_item_h + layout.toc_gap);
            let is_active = section == active_sec;

            if is_active {
                instances.push(GpuInstance {
                    rect: [layout.toc_x, y, layout.toc_w, layout.toc_item_h],
                    color: color::DUSK,
                });
            }

            let text_color = if is_active {
                color::CHAMPAGNE
            } else {
                color::MIST
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
        let smooth = self.scroll.tick();
        let scroll = smooth.floor() as usize;
        let frac_offset = -(smooth.fract()) * (layout.slot_h + layout.slot_gap);
        let track_h = (8.0 * layout.scale).max(4.0);
        let label_frac = 0.35;
        let slider_frac = 0.50;
        let slider_x = layout.content_x + layout.content_w * label_frac;
        let slider_w = layout.content_w * slider_frac;
        let pct_x = slider_x + slider_w + layout.content_w * 0.02;
        let pct_w = layout.content_w * 0.13;

        let render_slots = layout.visible_slots + 1;
        for (vi, ci) in (scroll..CONTENT.len()).enumerate() {
            if vi >= render_slots {
                break;
            }
            let slot_y = layout.content_start_y
                + vi as f32 * (layout.slot_h + layout.slot_gap)
                + frac_offset;
            match CONTENT[ci] {
                ContentSlot::Header(section) => {
                    // Gold section heading with a subtle underline.
                    text_labels.push(TextLabel {
                        rect: [layout.content_x, slot_y, layout.content_w, layout.slot_h],
                        text: section.label().into(),
                        color: color::GOLD,
                        align: TextAlign::Left,
                        ..Default::default()
                    });
                    let line_y = slot_y + layout.slot_h - 2.0 * layout.scale;
                    instances.push(GpuInstance {
                        rect: [
                            layout.content_x,
                            line_y,
                            layout.content_w,
                            (2.0 * layout.scale).max(1.0),
                        ],
                        color: color::DUSK,
                    });
                }
                ContentSlot::Row(row) => {
                    let is_focused = !self.back_focused && row == self.focused;
                    self.draw_row(
                        instances,
                        text_labels,
                        row,
                        [layout.content_x, slot_y, layout.content_w, layout.slot_h],
                        is_focused,
                        track_h,
                        pct_x,
                        pct_w,
                    );
                    buttons.push(ButtonDef::scene(
                        (layout.content_x, slot_y, layout.content_w, layout.slot_h),
                        row.click_id(),
                    ));
                }
            }
        }

        // ── Scroll indicator ───────────────────────────────────────────
        let max_scroll = self.scroll.max();
        if max_scroll > 0.0 {
            let indicator_x = layout.content_x + layout.content_w + (4.0 * layout.scale);
            let indicator_y = layout.content_start_y;
            let indicator_w = (3.0 * layout.scale).max(2.0);
            let indicator_h = layout.visible_slots as f32 * (layout.slot_h + layout.slot_gap);
            instances.push(GpuInstance {
                rect: [indicator_x, indicator_y, indicator_w, indicator_h],
                color: color::OBSIDIAN,
            });
            let thumb_h = (indicator_h * (layout.visible_slots as f32 / CONTENT.len() as f32))
                .max(12.0 * layout.scale);
            let thumb_y = indicator_y + (indicator_h - thumb_h) * (smooth / max_scroll);
            instances.push(GpuInstance {
                rect: [indicator_x, thumb_y, indicator_w, thumb_h],
                color: color::GOLD,
            });
        }

        // ── Back button ────────────────────────────────────────────────
        let back_bg = if self.back_focused {
            color::TWILIGHT
        } else {
            color::INDIGO
        };
        instances.push(GpuInstance {
            rect: [layout.back_x, layout.back_y, layout.back_w, layout.back_h],
            color: back_bg,
        });
        let back_text = if self.back_focused {
            color::CHAMPAGNE
        } else {
            color::MIST
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
            text: "Up/Down: navigate   Left/Right: adjust   Space: toggle/select".into(),
            color: color::SLATE,
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
        is_focused: bool,
        track_h: f32,
        pct_x: f32,
        pct_w: f32,
    ) {
        let [row_x, row_y, row_w, row_h] = rect;
        let scale = (row_h / 40.0).max(0.5);

        match row {
            Row::Master | Row::Music | Row::Sfx | Row::Gamma | Row::UiScale => {
                let value = self.slider_value(row).unwrap_or(0.0);
                let (lo, hi, _) = Self::slider_range(row);
                let fill_ratio = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
                let label = match row {
                    Row::Master => "Master Volume",
                    Row::Music => "Music Volume",
                    Row::Sfx => "SFX Volume",
                    Row::Gamma => "Gamma",
                    Row::UiScale => "UI Scale",
                    _ => unreachable!(),
                };

                let bg_color = if is_focused {
                    [0.20, 0.32, 0.50, 0.90]
                } else {
                    [0.12, 0.15, 0.24, 0.75]
                };
                instances.push(GpuInstance {
                    rect: [row_x, row_y, row_w, row_h],
                    color: bg_color,
                });

                let text_color = if is_focused {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    [0.6, 0.6, 0.7, 0.9]
                };
                let label_w = row_w * 0.35;
                text_labels.push(TextLabel {
                    rect: [row_x + 8.0 * scale, row_y, label_w - 8.0 * scale, row_h],
                    text: label.into(),
                    color: text_color,
                    ..Default::default()
                });

                let slider_x = row_x + label_w;
                let slider_w = row_w * 0.50;
                let track_y = row_y + (row_h - track_h) * 0.5;
                instances.push(GpuInstance {
                    rect: [slider_x, track_y, slider_w, track_h],
                    color: color::OBSIDIAN,
                });
                let fill_w = slider_w * fill_ratio;
                let fill_color = if is_focused {
                    color::GOLD
                } else {
                    color::BRASS
                };
                instances.push(GpuInstance {
                    rect: [slider_x, track_y, fill_w, track_h],
                    color: fill_color,
                });
                let knob_size = track_h * 2.5;
                let knob_x = slider_x + fill_w - knob_size * 0.5;
                let knob_y = track_y + (track_h - knob_size) * 0.5;
                let knob_color = if is_focused {
                    color::CHAMPAGNE
                } else {
                    color::PARCHMENT
                };
                instances.push(GpuInstance {
                    rect: [knob_x, knob_y, knob_size, knob_size],
                    color: knob_color,
                });
                let readout = match row {
                    Row::Gamma => format!("{:.2}", value),
                    Row::UiScale => format!("{:.0}%", value * 100.0),
                    _ => format!("{}%", (value * 100.0).round() as u32),
                };
                text_labels.push(TextLabel {
                    rect: [pct_x, row_y, pct_w, row_h],
                    text: readout,
                    color: text_color,
                    ..Default::default()
                });
            }
            _ => {
                // Toggle / cycle rows share the same visual pattern.
                let bg_color = if is_focused {
                    color::DUSK
                } else {
                    color::INDIGO
                };
                instances.push(GpuInstance {
                    rect: [row_x, row_y, row_w, row_h],
                    color: bg_color,
                });
                let text_color = if is_focused {
                    color::CHAMPAGNE
                } else {
                    color::MIST
                };
                let text = match row {
                    Row::SfxToggle => format!(
                        "Sound Effects: {}",
                        if self.sfx_enabled { "ON" } else { "OFF" }
                    ),
                    Row::SmokeQuality => {
                        format!("Smoke Quality: {}", self.smoke_quality.label())
                    }
                    Row::SmokeAmount => {
                        format!("Smoke Amount: {}", self.smoke_amount.label())
                    }
                    Row::Effects => {
                        format!("Effects: {}", self.effects_quality.label())
                    }
                    Row::Tile => format!("Tile Style: {}", self.tile_preset.label()),
                    Row::Tileset => format!("Tile Set: {}", self.tileset_name),
                    Row::Shadows => format!(
                        "Shadows: {}",
                        if self.shadows_enabled { "ON" } else { "OFF" }
                    ),
                    Row::Ssr => format!(
                        "Reflections: {}",
                        if self.ssr_enabled { "ON" } else { "OFF" }
                    ),
                    Row::Hdr => format!(
                        "HDR: {} (restart required)",
                        if self.hdr_enabled { "ON" } else { "OFF" }
                    ),
                    Row::SwapAb => format!("Swap A/B: {}", if self.swap_ab { "ON" } else { "OFF" }),
                    Row::AutoCashInOnFullStructure => format!(
                        "Auto Cash-In on Full Structure: {}",
                        if self.auto_cash_in_on_full_structure {
                            "ON"
                        } else {
                            "OFF"
                        }
                    ),
                    Row::Hints => {
                        format!("Hints: {}", if self.hints_enabled { "ON" } else { "OFF" })
                    }
                    _ => unreachable!(),
                };
                text_labels.push(TextLabel {
                    rect: [row_x, row_y, row_w, row_h],
                    text,
                    color: text_color,
                    ..Default::default()
                });
            }
        }
    }
}

// ── SceneBehavior (standalone options screen) ──────────────────────────

impl SceneBehavior for OptionsScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if self.update_input(
            ctx.actions,
            ctx.button_clicks,
            ctx.cursor_pos,
            ctx.layout.window_w,
            ctx.layout.window_h,
            ctx.scroll_lines,
        ) {
            if self.take_cancel_requested() {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
            }
            return Some(Scene::StartScreen(StartScreenScene::new()));
        }
        if self.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        if self.take_confirm_requested() {
            ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        }];
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();

        self.draw_overlay(w, h, &mut instances, &mut text_labels, &mut buttons);

        let mut frame = UiFrame::new();
        frame.quads(instances);
        frame.texts(text_labels);
        frame.buttons = buttons;
        frame.window_title = "Mahjuro — Options".into();
        frame
    }
}
