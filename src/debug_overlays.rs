//! Debug overlay panels: visibility toggles, cascade tuning, SFX test,
//! camera parameter editor.  All are App-owned modal overlays that
//! intercept input while open.

use crate::audio;
use crate::cascade_tuning_timeline::{
    apply_timeline_drag, draw_timelines, CascadeTuningTimelineGeom, TimelineDragTarget,
};
use crate::core::rules::ChamberKind;
use crate::game::cascade::CascadeTuning;
use crate::game::scene_look_tuning::{
    self, SCENE_LOOK_SLIDER_COUNT, SCENE_LOOK_SLIDER_META, SceneLookTuning, SceneLookTuningSet,
    hue_wheel_preview_linear,
};
use crate::game::tonemap_tuning::FALLBACK_SCENE_KEY;
use crate::render::draw_cmd::CameraParams;
use crate::render::hallway_glb::{
    HallwayDistortion, HallwayDistortionDebugSnapshot, hallway_distortion_apply_glb_depth_extent,
};
use crate::render::debug_overlay_ui::{self, DebugPointerState, DebugRowVisual};
use crate::render::theme::{color, metrics, typography, ButtonVariant};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::input::UiAction;
use sdl3::keyboard::Scancode;

// ── Debug visibility overlay ────────────────────────────────────────────

use crate::scenes::debug_visibility::{
    DebugVisibility, GAMEPLAY_VIS_ROW_COUNT, GAMEPLAY_VIS_VISIBLE_ROWS,
};

/// Cursor index for the Hide All / Reveal All button (one past the last checkbox row).
const VIS_CURSOR_TOGGLE_ALL: usize = GAMEPLAY_VIS_ROW_COUNT;

struct DebugVisLayout {
    panel_x: f32,
    panel_w: f32,
    rows_y0: f32,
    row_h: f32,
    row_gap: f32,
    btn_h: f32,
    visible_window: usize,
}

impl DebugVisLayout {
    fn compute(window_w: f32, window_h: f32) -> Self {
        let scale = metrics::scene_scale(window_w, window_h);
        let panel_w = (440.0 * scale).clamp(360.0, 680.0).min(window_w * 0.90);
        let row_font = typography::tier_at_most(26.0, window_h);
        let row_h = (row_font * 1.55).clamp(30.0, 44.0);
        let row_gap = (5.0 * scale).clamp(4.0, 6.0);
        let title_font = typography::tier_at_most(34.0, window_h);
        let title_h = (title_font * 1.45).clamp(34.0, 50.0);
        let btn_h = (row_font * 1.6).clamp(30.0, 44.0);
        let footer_font = (row_font * 0.70).max(12.0);
        let footer_h = (footer_font * 1.35).clamp(16.0, 24.0);
        let visible_rows = GAMEPLAY_VIS_VISIBLE_ROWS.min(GAMEPLAY_VIS_ROW_COUNT) as f32;
        let panel_h = title_h
            + row_gap
            + visible_rows * (row_h + row_gap)
            + row_gap
            + btn_h
            + footer_h
            + row_gap * 3.0;
        let panel_x = (window_w - panel_w) * 0.5;
        let panel_y = (window_h - panel_h) * 0.5;
        let rows_y0 = panel_y + row_gap + title_h + row_gap;
        Self {
            panel_x,
            panel_w,
            rows_y0,
            row_h,
            row_gap,
            btn_h,
            visible_window: GAMEPLAY_VIS_VISIBLE_ROWS.min(GAMEPLAY_VIS_ROW_COUNT),
        }
    }

    fn checkbox_row_rect(&self, slot: usize) -> (f32, f32, f32, f32) {
        let row_y = self.rows_y0 + slot as f32 * (self.row_h + self.row_gap);
        (self.panel_x + 4.0, row_y, self.panel_w - 8.0, self.row_h)
    }

    fn toggle_all_rect(&self) -> (f32, f32, f32, f32) {
        let row_y = self.rows_y0 + self.visible_window as f32 * (self.row_h + self.row_gap);
        let btn_y = row_y + self.row_gap;
        (self.panel_x + 4.0, btn_y, self.panel_w - 8.0, self.btn_h)
    }
}

#[inline]
fn debug_vis_point_in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx <= r.0 + r.2 && my >= r.1 && my <= r.1 + r.3
}

pub struct DebugVisibilityOverlay {
    cursor: usize,
    scroll: usize,
    pub vis: DebugVisibility,
    pointer: DebugPointerState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DebugVisResult {
    Stay,
    Close,
}

impl DebugVisibilityOverlay {
    pub fn new(vis: DebugVisibility) -> Self {
        Self {
            cursor: 0,
            scroll: 0,
            vis,
            pointer: DebugPointerState::default(),
        }
    }

    /// Total cursor positions = checkbox rows + 1 (the toggle-all button).
    const CURSOR_COUNT: usize = GAMEPLAY_VIS_ROW_COUNT + 1;

    pub fn update(
        &mut self,
        actions: &[UiAction],
        mouse: Option<(f32, f32, bool, bool)>,
        window_w: f32,
        window_h: f32,
    ) -> DebugVisResult {
        let layout = DebugVisLayout::compute(window_w, window_h);
        self.pointer.sync_held(mouse);
        self.pointer.clear_hover();

        if let Some((mx, my, clicked, _)) = mouse {
            let mut hit = false;
            for slot in 0..layout.visible_window {
                let i = self.scroll + slot;
                if i >= GAMEPLAY_VIS_ROW_COUNT {
                    break;
                }
                let rect = layout.checkbox_row_rect(slot);
                if debug_vis_point_in_rect(mx, my, rect) {
                    self.pointer.hover_row = Some(i);
                    self.cursor = i;
                    hit = true;
                    if clicked
                        && let Some(f) = self.vis.flag_mut(i) {
                            *f = !*f;
                        }
                    break;
                }
            }
            if !hit && debug_vis_point_in_rect(mx, my, layout.toggle_all_rect()) {
                self.pointer.hover_row = Some(VIS_CURSOR_TOGGLE_ALL);
                self.cursor = VIS_CURSOR_TOGGLE_ALL;
                if clicked {
                    let hide_all = !self.vis.any_hide();
                    for i in 0..GAMEPLAY_VIS_ROW_COUNT {
                        if let Some(f) = self.vis.flag_mut(i) {
                            *f = hide_all;
                        }
                    }
                }
            }
        }

        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % Self::CURSOR_COUNT;
                    self.clamp_scroll();
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + Self::CURSOR_COUNT - 1) % Self::CURSOR_COUNT;
                    self.clamp_scroll();
                }
                UiAction::Confirm => {
                    if self.cursor == VIS_CURSOR_TOGGLE_ALL {
                        let hide_all = !self.vis.any_hide();
                        for i in 0..GAMEPLAY_VIS_ROW_COUNT {
                            if let Some(f) = self.vis.flag_mut(i) {
                                *f = hide_all;
                            }
                        }
                    } else if let Some(f) = self.vis.flag_mut(self.cursor) {
                        *f = !*f;
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    return DebugVisResult::Close;
                }
                _ => {}
            }
        }
        DebugVisResult::Stay
    }

    fn clamp_scroll(&mut self) {
        // The toggle-all button sits outside the scrollable list; nothing to scroll.
        if self.cursor == VIS_CURSOR_TOGGLE_ALL {
            return;
        }
        let window = GAMEPLAY_VIS_VISIBLE_ROWS.min(GAMEPLAY_VIS_ROW_COUNT);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + window {
            self.scroll = self.cursor + 1 - window;
        }
    }

    pub fn draw(&self, window_w: f32, window_h: f32) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let scale = metrics::scene_scale(window_w, window_h);
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        // Dim full-screen backdrop.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: color::alpha(color::WALNUT_INK, 0.7),
            user: 0,
        });

        let panel_w = (440.0 * scale).clamp(360.0, 680.0).min(window_w * 0.90);
        let title_font = typography::tier_at_most(34.0, window_h);
        let row_font = typography::tier_at_most(26.0, window_h);
        let footer_font = (row_font * 0.70).max(12.0);
        let row_h = (row_font * 1.55).clamp(30.0, 44.0);
        let row_gap = (5.0 * scale).clamp(4.0, 6.0);
        let title_h = (title_font * 1.45).clamp(34.0, 50.0);
        let footer_h = (footer_font * 1.35).clamp(16.0, 24.0);
        let visible_rows = GAMEPLAY_VIS_VISIBLE_ROWS.min(GAMEPLAY_VIS_ROW_COUNT) as f32;
        let btn_h = (row_font * 1.6).clamp(30.0, 44.0);
        let panel_h = title_h
            + row_gap
            + visible_rows * (row_h + row_gap)
            + row_gap
            + btn_h
            + footer_h
            + row_gap * 3.0;
        let panel_x = (window_w - panel_w) * 0.5;
        let panel_y = (window_h - panel_h) * 0.5;

        // Border.
        let border = 3.0;
        instances.push(GpuInstance {
            rect: [
                panel_x - border,
                panel_y - border,
                panel_w + border * 2.0,
                panel_h + border * 2.0,
            ],
            color: color::alpha(color::WALNUT_BRIGHT, 0.85),
            user: 0,
        });
        // Panel.
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.95),
            user: 0,
        });

        // Title.
        labels.push(TextLabel {
            rect: [panel_x, panel_y + row_gap, panel_w, title_h],
            text: "Debug Visibility".into(),
            color: color::PARCHMENT,
            font_px: Some(title_font),
            ..Default::default()
        });

        let mut row_y = panel_y + row_gap + title_h + row_gap;
        let row_pad = (12.0 * scale).clamp(10.0, 18.0);
        let check_size = (row_h * 0.48).clamp(16.0, 22.0);
        let window = GAMEPLAY_VIS_VISIBLE_ROWS.min(GAMEPLAY_VIS_ROW_COUNT);
        let first = self.scroll.min(GAMEPLAY_VIS_ROW_COUNT.saturating_sub(1));
        for slot in 0..window {
            let i = first + slot;
            if i >= GAMEPLAY_VIS_ROW_COUNT {
                break;
            }
            let (name, checked) = self.vis.label_checked(i);
            let visual = DebugRowVisual::for_row(i, self.cursor, &self.pointer);

            // Row background.
            let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, ButtonVariant::Default);
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h],
                color: bg,
                user: 0,
            });

            // Checkbox border.
            let cb_x = panel_x + row_pad;
            let cb_y = row_y + (row_h - check_size) * 0.5;
            instances.push(GpuInstance {
                rect: [cb_x - 2.0, cb_y - 2.0, check_size + 4.0, check_size + 4.0],
                color: color::alpha(color::STONE, 0.9),
                user: 0,
            });
            instances.push(GpuInstance {
                rect: [cb_x, cb_y, check_size, check_size],
                color: color::WALNUT_INK,
                user: 0,
            });
            // Filled square when checked.
            if checked {
                let pad = check_size * 0.18;
                instances.push(GpuInstance {
                    rect: [
                        cb_x + pad,
                        cb_y + pad,
                        check_size - pad * 2.0,
                        check_size - pad * 2.0,
                    ],
                    color: color::GOLD,
                    user: 0,
                });
            }

            // Label.
            labels.push(TextLabel {
                rect: [
                    cb_x + check_size + row_pad,
                    row_y + (row_h - row_font * 1.25) * 0.5,
                    panel_w - (check_size + row_pad * 3.0),
                    row_font * 1.25,
                ],
                text: name.to_string(),
                color: tc,
                font_px: Some(row_font),
                align: TextAlign::Left,
                ..Default::default()
            });

            row_y += row_h + row_gap;
        }

        // Hide All / Reveal All button.
        let btn_visual =
            DebugRowVisual::for_row(VIS_CURSOR_TOGGLE_ALL, self.cursor, &self.pointer);
        let all_hidden = self.vis.any_hide();
        let btn_label = if all_hidden { "Reveal All" } else { "Hide All" };
        let (btn_bg, btn_tc) =
            debug_overlay_ui::row_surface_colors(btn_visual, ButtonVariant::Primary);
        let btn_y = row_y + row_gap;
        instances.push(GpuInstance {
            rect: [panel_x + 4.0, btn_y, panel_w - 8.0, btn_h],
            color: btn_bg,
            user: 0,
        });
        labels.push(TextLabel {
            rect: [panel_x + 4.0, btn_y, panel_w - 8.0, btn_h],
            text: btn_label.into(),
            color: btn_tc,
            font_px: Some(row_font),
            bold: btn_visual.highlighted,
            ..Default::default()
        });

        // Footer hint.
        labels.push(TextLabel {
            rect: [panel_x, btn_y + btn_h + row_gap, panel_w, footer_h],
            text: "Up/Down select   Enter toggle / activate   Esc close".into(),
            color: color::alpha(color::STONE, 0.75),
            font_px: Some(footer_font),
            ..Default::default()
        });

        (instances, labels)
    }
}

// ── Cascade tuning overlay ──────────────────────────────────────────────

pub const TUNING_ROW_COUNT: usize = 13; // 12 sliders + Export button
const TUNING_SLIDER_ROWS: usize = TUNING_ROW_COUNT - 1;
const TUNING_MIN_MS: u64 = 50;
const TUNING_MAX_MS: u64 = 5000;
const TUNING_STEP_MS: u64 = 50;

struct TuningLayout {
    panel_x: f32,
    panel_y: f32,
    panel_w: f32,
    rows_y0: f32,
    row_h: f32,
    row_gap: f32,
    title_h: f32,
    row_total_h: f32,
    label_w: f32,
    slider_w: f32,
    scale: f32,
}

impl TuningLayout {
    fn compute(window_w: f32, window_h: f32) -> Self {
        let scale = metrics::scene_scale(window_w, window_h);
        let panel_w = (520.0 * scale).min(window_w * 0.90);
        let row_h = (40.0 * scale).max(26.0);
        let desc_h = (18.0 * scale).max(12.0);
        let row_gap = (10.0 * scale).max(4.0);
        let title_h = (48.0 * scale).max(28.0);
        let diagram_h = (120.0 * scale).max(88.0);
        let row_total_h = row_h + desc_h + row_gap;
        let panel_h = title_h
            + row_gap
            + diagram_h
            + row_gap
            + TUNING_SLIDER_ROWS as f32 * row_total_h
            + (row_h + row_gap)
            + row_gap * 3.0;
        let panel_x = (window_w - panel_w) * 0.5;
        let panel_y = (window_h - panel_h) * 0.5;
        let rows_y0 = panel_y + row_gap + title_h + row_gap + diagram_h + row_gap;
        let label_w = panel_w * 0.38;
        let slider_w = panel_w * 0.35;
        Self {
            panel_x,
            panel_y,
            panel_w,
            rows_y0,
            row_h,
            row_gap,
            title_h,
            row_total_h,
            label_w,
            slider_w,
            scale,
        }
    }

    fn title_block_h(&self) -> f32 {
        self.row_gap + self.title_h + self.row_gap
    }

    fn slider_track(&self, row: usize) -> (f32, f32, f32, f32) {
        let row_y = self.rows_y0 + row as f32 * self.row_total_h;
        let track_x = self.panel_x + self.label_w;
        let track_h = (8.0 * self.scale).max(4.0);
        let track_y = row_y + (self.row_h - track_h) * 0.5;
        (track_x, track_y, self.slider_w, track_h)
    }

    fn export_button_rect(&self) -> (f32, f32, f32, f32) {
        let export_y = self.rows_y0 + TUNING_SLIDER_ROWS as f32 * self.row_total_h;
        (self.panel_x + 4.0, export_y, self.panel_w - 8.0, self.row_h)
    }

    fn row_rect(&self, row: usize) -> (f32, f32, f32, f32) {
        if row >= TUNING_ROW_COUNT - 1 {
            return self.export_button_rect();
        }
        let desc_h = self.row_total_h - self.row_h - self.row_gap;
        let row_y = self.rows_y0 + row as f32 * self.row_total_h;
        (
            self.panel_x + 4.0,
            row_y,
            self.panel_w - 8.0,
            self.row_h + desc_h,
        )
    }
}

#[inline]
fn tuning_point_in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx <= r.0 + r.2 && my >= r.1 && my <= r.1 + r.3
}

pub struct TuningOverlay {
    cursor: usize,
    pub tuning: CascadeTuning,
    dragging_slider: Option<usize>,
    dragging_timeline: Option<TimelineDragTarget>,
    pointer: DebugPointerState,
}

pub enum TuningResult {
    Stay,
    Close,
    Export,
}

impl TuningOverlay {
    pub fn new(tuning: &CascadeTuning) -> Self {
        Self {
            cursor: 0,
            tuning: tuning.clone(),
            dragging_slider: None,
            dragging_timeline: None,
            pointer: DebugPointerState::default(),
        }
    }

    fn apply_slider_mx(&mut self, row: usize, mx: f32, layout: &TuningLayout) {
        let (tx, _, tw, _) = layout.slider_track(row);
        let t = ((mx - tx) / tw.max(1e-6)).clamp(0.0, 1.0);
        let ms = (TUNING_MIN_MS as f32 + t * (TUNING_MAX_MS - TUNING_MIN_MS) as f32).round() as u64;
        self.set_row_ms(row, ms);
    }

    fn set_row_ms(&mut self, row: usize, ms: u64) {
        let ms = ms.clamp(TUNING_MIN_MS, TUNING_MAX_MS);
        let field = match row {
            0 => &mut self.tuning.base_hold_ms,
            1 => &mut self.tuning.step_hold_ms,
            2 => &mut self.tuning.total_hold_ms,
            3 => &mut self.tuning.tick_duration_ms,
            4 => &mut self.tuning.discard_refill_cap_ms,
            5 => &mut self.tuning.discard_lift_ms,
            6 => &mut self.tuning.discard_flight_ms,
            7 => &mut self.tuning.discard_landing_ms,
            8 => &mut self.tuning.discard_stagger_ms,
            9 => &mut self.tuning.discard_river_sink_ms,
            10 => &mut self.tuning.draw_settle_ms,
            11 => &mut self.tuning.sort_settle_ms,
            _ => return,
        };
        *field = ms;
    }

    pub fn update(
        &mut self,
        actions: &[UiAction],
        mouse: Option<(f32, f32, bool, bool)>,
        window_w: f32,
        window_h: f32,
    ) -> TuningResult {
        let layout = TuningLayout::compute(window_w, window_h);
        self.pointer.sync_held(mouse);
        self.pointer.clear_hover();

        if let Some((mx, my, clicked, held)) = mouse {
            let diagram_top = layout.panel_y + layout.title_block_h();
            let timeline = CascadeTuningTimelineGeom::compute(
                layout.panel_x,
                layout.panel_w,
                diagram_top,
                layout.scale,
            );

            if clicked && self.dragging_timeline.is_none() && self.dragging_slider.is_none()
                && let Some(target) = timeline.hit_handle(mx, my, &self.tuning) {
                    self.dragging_timeline = Some(target);
                    self.cursor = CascadeTuningTimelineGeom::cursor_for_drag(target);
                }
            if let Some(target) = self.dragging_timeline
                && held {
                    apply_timeline_drag(&mut self.tuning, target, mx, &timeline);
                    self.cursor = CascadeTuningTimelineGeom::cursor_for_drag(target);
                }

            for i in 0..TUNING_ROW_COUNT {
                if debug_overlay_ui::point_in_rect_tuple(mx, my, layout.row_rect(i)) {
                    self.pointer.hover_row = Some(i);
                    if self.dragging_slider.is_none() && self.dragging_timeline.is_none() {
                        self.cursor = i;
                    }
                    break;
                }
            }

            if let Some(di) = self.dragging_slider
                && held
                && di < TUNING_SLIDER_ROWS
            {
                self.apply_slider_mx(di, mx, &layout);
            }

            if (clicked || held)
                && self.dragging_slider.is_none()
                && self.dragging_timeline.is_none()
            {
                for i in 0..TUNING_SLIDER_ROWS {
                    if tuning_point_in_rect(mx, my, layout.slider_track(i)) {
                        self.cursor = i;
                        self.apply_slider_mx(i, mx, &layout);
                        if held {
                            self.dragging_slider = Some(i);
                        }
                        break;
                    }
                }
            }

            if clicked && self.dragging_slider.is_none() && self.dragging_timeline.is_none()
                && tuning_point_in_rect(mx, my, layout.export_button_rect()) {
                    self.cursor = TUNING_ROW_COUNT - 1;
                    return TuningResult::Export;
                }
        }

        if let Some((_, _, _, held)) = mouse {
            if !held {
                self.dragging_slider = None;
                self.dragging_timeline = None;
            }
        } else {
            self.dragging_slider = None;
            self.dragging_timeline = None;
        }

        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % TUNING_ROW_COUNT;
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + TUNING_ROW_COUNT - 1) % TUNING_ROW_COUNT;
                }
                UiAction::FocusNext | UiAction::NavigateHudNext => {
                    self.adjust(TUNING_STEP_MS as i64);
                }
                UiAction::FocusPrev | UiAction::NavigateHudPrev => {
                    self.adjust(-(TUNING_STEP_MS as i64));
                }
                UiAction::Confirm => {
                    if self.cursor == TUNING_ROW_COUNT - 1 {
                        return TuningResult::Export;
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    return TuningResult::Close;
                }
                _ => {}
            }
        }
        TuningResult::Stay
    }

    fn adjust(&mut self, delta: i64) {
        let field = match self.cursor {
            0 => &mut self.tuning.base_hold_ms,
            1 => &mut self.tuning.step_hold_ms,
            2 => &mut self.tuning.total_hold_ms,
            3 => &mut self.tuning.tick_duration_ms,
            4 => &mut self.tuning.discard_refill_cap_ms,
            5 => &mut self.tuning.discard_lift_ms,
            6 => &mut self.tuning.discard_flight_ms,
            7 => &mut self.tuning.discard_landing_ms,
            8 => &mut self.tuning.discard_stagger_ms,
            9 => &mut self.tuning.discard_river_sink_ms,
            10 => &mut self.tuning.draw_settle_ms,
            11 => &mut self.tuning.sort_settle_ms,
            _ => return,
        };
        *field = (*field as i64 + delta).clamp(TUNING_MIN_MS as i64, TUNING_MAX_MS as i64) as u64;
    }

    pub fn draw(&self, window_w: f32, window_h: f32) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let scale = metrics::scene_scale(window_w, window_h);
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        // Dim overlay background.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: color::alpha(color::WALNUT_INK, 0.7),
            user: 0,
        });

        // Panel dimensions.
        let panel_w = (520.0 * scale).min(window_w * 0.90);
        let row_h = (40.0 * scale).max(26.0);
        let desc_h = (18.0 * scale).max(12.0);
        let row_gap = (10.0 * scale).max(4.0);
        let title_h = (48.0 * scale).max(28.0);
        let diagram_h = (120.0 * scale).max(88.0);
        let row_total_h = row_h + desc_h + row_gap;
        let panel_h = title_h + row_gap
            + diagram_h + row_gap
            + TUNING_SLIDER_ROWS as f32 * row_total_h
            + (row_h + row_gap)  // export button
            + row_gap * 3.0;
        let panel_x = (window_w - panel_w) * 0.5;
        let panel_y = (window_h - panel_h) * 0.5;

        // Panel background.
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.95),
            user: 0,
        });
        // Panel border.
        let border = 3.0;
        instances.push(GpuInstance {
            rect: [
                panel_x - border,
                panel_y - border,
                panel_w + border * 2.0,
                panel_h + border * 2.0,
            ],
            color: color::alpha(color::WALNUT_BRIGHT, 0.8),
            user: 0,
        });
        // Re-draw panel on top of border.
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.95),
            user: 0,
        });

        // Title.
        labels.push(TextLabel {
            rect: [panel_x, panel_y + row_gap, panel_w, title_h],
            text: "Cascade Tuning".into(),
            color: color::PARCHMENT,
            ..Default::default()
        });

        let diagram_top = panel_y + row_gap + title_h + row_gap;
        let timeline_geom =
            CascadeTuningTimelineGeom::compute(panel_x, panel_w, diagram_top, scale);
        draw_timelines(
            &timeline_geom,
            &self.tuning,
            panel_x,
            panel_w,
            scale,
            self.cursor,
            self.dragging_timeline,
            window_h,
            &mut instances,
            &mut labels,
        );

        let cursor_y = diagram_top + timeline_geom.diagram_h + row_gap;

        // Slider rows with descriptions.
        let label_w = panel_w * 0.38;
        let slider_w = panel_w * 0.35;
        let value_w = panel_w * 0.18;

        let rows: [(&str, &str, u64); TUNING_SLIDER_ROWS] = [
            (
                "Base Hold",
                "Pause on base points before steps begin",
                self.tuning.base_hold_ms,
            ),
            (
                "Step Hold",
                "Pause per relic/rule multiplier step",
                self.tuning.step_hold_ms,
            ),
            (
                "Total Hold",
                "Pause on final total before resuming play",
                self.tuning.total_hold_ms,
            ),
            (
                "Tick Duration",
                "Speed of the score counter tick-up animation",
                self.tuning.tick_duration_ms,
            ),
            (
                "Discard Refill Cap",
                "Max wait before auto-draw after discard (fallback ceiling)",
                self.tuning.discard_refill_cap_ms,
            ),
            (
                "Discard Lift",
                "Per-tile rise above the hand before the arc",
                self.tuning.discard_lift_ms,
            ),
            (
                "Discard Flight",
                "Curved arc from hand into the river",
                self.tuning.discard_flight_ms,
            ),
            (
                "Discard Landing",
                "Small settle on the river surface",
                self.tuning.discard_landing_ms,
            ),
            (
                "Discard Stagger",
                "Random spread for when each tile starts (capped 8–200ms)",
                self.tuning.discard_stagger_ms,
            ),
            (
                "River Sink",
                "Previous river pile sinks before despawn",
                self.tuning.discard_river_sink_ms,
            ),
            (
                "Draw Speed",
                "How long drawn tiles take to settle in",
                self.tuning.draw_settle_ms,
            ),
            (
                "Sort/Drag Speed",
                "How long sort and drag-reorder animations take",
                self.tuning.sort_settle_ms,
            ),
        ];

        for (i, (name, desc, value)) in rows.iter().enumerate() {
            let row_y = cursor_y + i as f32 * row_total_h;
            let visual = DebugRowVisual::for_row(i, self.cursor, &self.pointer);
            let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, ButtonVariant::Default);

            // Row background.
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h + desc_h],
                color: bg,
                user: 0,
            });

            // Label.
            labels.push(TextLabel {
                rect: [panel_x + 12.0 * scale, row_y, label_w, row_h],
                text: name.to_string(),
                color: tc,
                ..Default::default()
            });

            // Description below label.
            labels.push(TextLabel {
                rect: [
                    panel_x + 12.0 * scale,
                    row_y + row_h * 0.75,
                    label_w + slider_w,
                    desc_h,
                ],
                text: desc.to_string(),
                color: color::alpha(color::UMBER, 0.85),
                ..Default::default()
            });

            // Slider track.
            let track_x = panel_x + label_w;
            let track_h = (8.0 * scale).max(4.0);
            let track_y = row_y + (row_h - track_h) * 0.5;
            instances.push(GpuInstance {
                rect: [track_x, track_y, slider_w, track_h],
                color: color::WALNUT_INK,
                user: 0,
            });

            // Slider fill.
            let t = (*value as f32 - TUNING_MIN_MS as f32) / (TUNING_MAX_MS - TUNING_MIN_MS) as f32;
            let fill_w = slider_w * t.clamp(0.0, 1.0);
            let fill_color = debug_overlay_ui::slider_accent_color(visual);
            instances.push(GpuInstance {
                rect: [track_x, track_y, fill_w, track_h],
                color: fill_color,
                user: 0,
            });

            // Knob.
            let knob_size = track_h * 2.5;
            let knob_x = track_x + fill_w - knob_size * 0.5;
            let knob_y = track_y + (track_h - knob_size) * 0.5;
            instances.push(GpuInstance {
                rect: [knob_x, knob_y, knob_size, knob_size],
                color: if visual.highlighted {
                    color::PARCHMENT
                } else {
                    color::alpha(color::STONE, 0.9)
                },
                user: 0,
            });

            // Value text.
            let value_x = panel_x + label_w + slider_w + 4.0;
            labels.push(TextLabel {
                rect: [value_x, row_y, value_w, row_h],
                text: format!("{}ms", value),
                color: tc,
                ..Default::default()
            });
        }

        // Export button row.
        let export_y = cursor_y + TUNING_SLIDER_ROWS as f32 * row_total_h;
        let export_visual =
            DebugRowVisual::for_row(TUNING_ROW_COUNT - 1, self.cursor, &self.pointer);
        let (export_bg, export_tc) =
            debug_overlay_ui::row_surface_colors(export_visual, ButtonVariant::Primary);
        instances.push(GpuInstance {
            rect: [panel_x + 4.0, export_y, panel_w - 8.0, row_h],
            color: export_bg,
            user: 0,
        });
        labels.push(TextLabel {
            rect: [panel_x, export_y, panel_w, row_h],
            text: "Export as JSON".into(),
            color: export_tc,
            ..Default::default()
        });

        // Hint.
        let hint_y = export_y + row_h + row_gap;
        labels.push(TextLabel {
            rect: [panel_x, hint_y, panel_w, row_h * 0.6],
            text: "Drag timeline handles · sliders below · Left/Right nudge · Esc close".into(),
            color: color::alpha(color::UMBER, 0.7),
            ..Default::default()
        });

        (instances, labels)
    }
}

// ── Sound effects test overlay ──────────────────────────────────────────

pub struct SfxTestOverlay {
    cursor: usize,
    /// Cached row rects from the last `draw()` call, used for mouse hit-testing.
    row_rects: Vec<[f32; 4]>,
    pointer: DebugPointerState,
}

impl SfxTestOverlay {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            row_rects: Vec::new(),
            pointer: DebugPointerState::default(),
        }
    }

    /// Returns `true` if the overlay should close.
    ///
    /// `mouse` is `Some((x, y, clicked, held))` when cursor position is known;
    /// `clicked` is true on the frame a left-button press landed.
    pub fn update(
        &mut self,
        actions: &[UiAction],
        audio: &mut audio::AudioManager,
        mouse: Option<(f32, f32, bool, bool)>,
    ) -> bool {
        let count = audio::all_sfx_ids().len();
        self.pointer.sync_held(mouse);
        self.pointer.clear_hover();

        // Mouse hover → move cursor; click → play.
        if let Some((mx, my, clicked, _)) = mouse {
            for (i, r) in self.row_rects.iter().enumerate() {
                if debug_overlay_ui::point_in_rect(mx, my, *r) {
                    self.pointer.hover_row = Some(i);
                    self.cursor = i;
                    if clicked && let Some(&id) = audio::all_sfx_ids().get(i) {
                        audio.play_sfx(id);
                    }
                    break;
                }
            }
        }

        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % count;
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + count - 1) % count;
                }
                UiAction::Confirm | UiAction::CommitDiscard => {
                    if let Some(&id) = audio::all_sfx_ids().get(self.cursor) {
                        audio.play_sfx(id);
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    pub fn draw(&mut self, window_w: f32, window_h: f32) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let scale = metrics::scene_scale(window_w, window_h);
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        // Dim overlay background.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: color::alpha(color::WALNUT_INK, 0.7),
            user: 0,
        });

        let ids = audio::all_sfx_ids();
        let row_h = (36.0 * scale).max(24.0);
        let row_gap = (6.0 * scale).max(3.0);
        let title_h = (48.0 * scale).max(28.0);
        let hint_h = (22.0 * scale).max(14.0);
        let pad = (16.0 * scale).max(8.0);

        let panel_w = (560.0 * scale).min(window_w * 0.92);
        let panel_h =
            pad + title_h + pad + (ids.len() as f32) * (row_h + row_gap) + pad + hint_h + pad;
        let panel_x = (window_w - panel_w) * 0.5;
        let panel_y = (window_h - panel_h) * 0.5;

        // Border.
        let border = 3.0;
        instances.push(GpuInstance {
            rect: [
                panel_x - border,
                panel_y - border,
                panel_w + border * 2.0,
                panel_h + border * 2.0,
            ],
            color: color::alpha(color::ANTIQUE, 0.85),
            user: 0,
        });
        // Panel background (theme WALNUT_DEEP — dark walnut).
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.97),
            user: 0,
        });

        // Title.
        labels.push(TextLabel {
            rect: [panel_x, panel_y + pad, panel_w, title_h],
            text: "Sound Effects Test".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // Rows — also cache hit-test rects for mouse support.
        self.row_rects.clear();
        let rows_y0 = panel_y + pad + title_h + pad;
        for (i, &id) in ids.iter().enumerate() {
            let row_y = rows_y0 + i as f32 * (row_h + row_gap);
            self.row_rects
                .push([panel_x + 4.0, row_y, panel_w - 8.0, row_h]);
            let visual = DebugRowVisual::for_row(i, self.cursor, &self.pointer);
            let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, ButtonVariant::Default);
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h],
                color: bg,
                user: 0,
            });

            // Variant name (left).
            let name_w = panel_w * 0.32;
            labels.push(TextLabel {
                rect: [panel_x + 12.0 * scale, row_y, name_w, row_h],
                text: format!("{:?}", id),
                color: tc,
                ..Default::default()
            });
            // Filename (right).
            labels.push(TextLabel {
                rect: [
                    panel_x + 12.0 * scale + name_w,
                    row_y,
                    panel_w - name_w - 24.0 * scale,
                    row_h,
                ],
                text: id.filename().to_string(),
                color: [tc[0] * 0.85, tc[1] * 0.85, tc[2] * 0.85, tc[3] * 0.9],
                ..Default::default()
            });
        }

        // Footer hint.
        let hint_y = rows_y0 + ids.len() as f32 * (row_h + row_gap) + pad;
        labels.push(TextLabel {
            rect: [panel_x, hint_y, panel_w, hint_h],
            text: "Up/Down: select   Enter/Click: play   Esc: close".into(),
            color: color::alpha(color::STONE, 0.75),
            ..Default::default()
        });

        (instances, labels)
    }
}

// ── Camera debug overlay ────────────────────────────────────────────────

pub struct CameraDebugOverlay {
    cursor: usize,
    eye: [f32; 3],
    target: [f32; 3],
    up: [f32; 3],
    fovy_deg: f32,
    /// Window height at draw time, included in clipboard output so the
    /// camera can be scaled proportionally at different resolutions.
    last_window_h: f32,
    pointer: DebugPointerState,
}

impl CameraDebugOverlay {
    pub fn new(cam: &CameraParams) -> Self {
        Self {
            cursor: 0,
            eye: cam.eye,
            target: cam.target,
            up: cam.up,
            fovy_deg: cam.fovy_deg,
            last_window_h: 0.0,
            pointer: DebugPointerState::default(),
        }
    }

    pub fn to_camera_params(&self) -> CameraParams {
        CameraParams {
            eye: self.eye,
            target: self.target,
            up: self.up,
            fovy_deg: self.fovy_deg,
            clip_near: None,
            clip_far: None,
        }
    }

    fn rows(&self) -> Vec<(&'static str, f32)> {
        vec![
            ("Eye X", self.eye[0]),
            ("Eye Y", self.eye[1]),
            ("Eye Z", self.eye[2]),
            ("Target X", self.target[0]),
            ("Target Y", self.target[1]),
            ("Target Z", self.target[2]),
            ("FOV (deg)", self.fovy_deg),
        ]
    }

    fn row_count(&self) -> usize {
        7
    }

    fn adjust(&mut self, delta: f32) {
        match self.cursor {
            0 => self.eye[0] += delta,
            1 => self.eye[1] += delta,
            2 => self.eye[2] += delta,
            3 => self.target[0] += delta,
            4 => self.target[1] += delta,
            5 => self.target[2] += delta,
            6 => self.fovy_deg = (self.fovy_deg + delta * 0.1).clamp(5.0, 120.0),
            _ => {}
        }
    }

    /// Returns `true` if the overlay should close.
    pub fn update(
        &mut self,
        actions: &[UiAction],
        mouse: Option<(f32, f32, bool, bool)>,
        window_w: f32,
        window_h: f32,
    ) -> bool {
        self.last_window_h = window_h;
        self.pointer.sync_held(mouse);
        self.pointer.clear_hover();

        let scale = metrics::scene_scale(window_w, window_h);
        let row_h = (22.0 * scale).max(16.0);
        let row_gap = (3.0 * scale).max(2.0);
        let title_h = (24.0 * scale).max(16.0);
        let pad = (8.0 * scale).max(5.0);
        let margin = (10.0 * scale).max(6.0);
        let panel_w = (260.0 * scale).min(window_w * 0.38);
        let panel_x = window_w - panel_w - margin;
        let panel_y = margin;
        let rows_y0 = panel_y + pad + title_h + pad;

        if let Some((mx, my, _, _)) = mouse {
            for i in 0..self.row_count() {
                let row_y = rows_y0 + i as f32 * (row_h + row_gap);
                let rect = [panel_x + 4.0, row_y, panel_w - 8.0, row_h];
                if debug_overlay_ui::point_in_rect(mx, my, rect) {
                    self.pointer.hover_row = Some(i);
                    self.cursor = i;
                    break;
                }
            }
        }

        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % self.row_count();
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + self.row_count() - 1) % self.row_count();
                }
                UiAction::FocusNext => {
                    self.adjust(10.0);
                }
                UiAction::FocusPrev => {
                    self.adjust(-10.0);
                }
                UiAction::Confirm | UiAction::CommitDiscard => {
                    // Copy to clipboard.  Includes the reference window
                    // height and pre-written `cs` scaling so the camera
                    // can be resolution-independent when pasted into a
                    // scene.
                    let rh = self.last_window_h;
                    let text = format!(
                        "// ref_h: {:.0}\nlet cs = h / {:.0}_f32;\nCameraParams {{\n    eye: [{:.1} * cs, {:.1} * cs, {:.1} * cs],\n    target: [{:.1} * cs, {:.1} * cs, {:.1} * cs],\n    up: [{:.1}, {:.1}, {:.1}],\n    fovy_deg: {:.1},\n}}",
                        rh,
                        rh,
                        self.eye[0],
                        self.eye[1],
                        self.eye[2],
                        self.target[0],
                        self.target[1],
                        self.target[2],
                        self.up[0],
                        self.up[1],
                        self.up[2],
                        self.fovy_deg,
                    );
                    match arboard::Clipboard::new() {
                        Ok(mut cb) => {
                            if let Err(e) = cb.set_text(&text) {
                                log::error!("Clipboard write failed: {e}");
                            } else {
                                log::info!("Camera params copied to clipboard");
                            }
                        }
                        Err(e) => log::error!("Could not open clipboard: {e}"),
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    pub fn draw(&self, window_w: f32, window_h: f32) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let scale = metrics::scene_scale(window_w, window_h);
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        let row_h = (22.0 * scale).max(16.0);
        let row_gap = (3.0 * scale).max(2.0);
        let title_h = (24.0 * scale).max(16.0);
        let hint_h = (14.0 * scale).max(10.0);
        let pad = (8.0 * scale).max(5.0);
        let margin = (10.0 * scale).max(6.0);
        let rows = self.rows();

        let panel_w = (260.0 * scale).min(window_w * 0.38);
        let panel_h =
            pad + title_h + pad + (rows.len() as f32) * (row_h + row_gap) + pad + hint_h + pad;
        let panel_x = window_w - panel_w - margin;
        let panel_y = margin;

        // Border.
        let border = 2.0;
        instances.push(GpuInstance {
            rect: [
                panel_x - border,
                panel_y - border,
                panel_w + border * 2.0,
                panel_h + border * 2.0,
            ],
            color: color::alpha(color::ANTIQUE, 0.85),
            user: 0,
        });
        // Panel background.
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.97),
            user: 0,
        });

        // Title.
        labels.push(TextLabel {
            rect: [panel_x, panel_y + pad, panel_w, title_h],
            text: "Camera Debug".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // Rows.
        let rows_y0 = panel_y + pad + title_h + pad;
        for (i, (name, value)) in rows.iter().enumerate() {
            let row_y = rows_y0 + i as f32 * (row_h + row_gap);
            let visual = DebugRowVisual::for_row(i, self.cursor, &self.pointer);
            let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, ButtonVariant::Default);
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h],
                color: bg,
                user: 0,
            });

            // Label (left).
            let name_w = panel_w * 0.50;
            labels.push(TextLabel {
                rect: [panel_x + 6.0 * scale, row_y, name_w, row_h],
                text: name.to_string(),
                color: tc,
                ..Default::default()
            });
            // Value (right).
            labels.push(TextLabel {
                rect: [
                    panel_x + 6.0 * scale + name_w,
                    row_y,
                    panel_w - name_w - 12.0 * scale,
                    row_h,
                ],
                text: format!("{:.1}", value),
                color: [tc[0] * 0.85, tc[1] * 0.85, tc[2] * 0.85, tc[3] * 0.9],
                ..Default::default()
            });
        }

        // Footer hint.
        let hint_y = rows_y0 + rows.len() as f32 * (row_h + row_gap) + pad;
        labels.push(TextLabel {
            rect: [panel_x, hint_y, panel_w, hint_h],
            text: "\u{2191}\u{2193} sel  \u{2190}\u{2192} adj  \u{23ce} copy  Esc".into(),
            color: color::alpha(color::STONE, 0.75),
            ..Default::default()
        });

        (instances, labels)
    }
}

// ── Per-scene look: tonemap / post-FX + room GLB lighting ─────────────────

const SCENE_LOOK_SAVE_ROW: usize = SCENE_LOOK_SLIDER_COUNT;
const SCENE_LOOK_RESET_ROW: usize = SCENE_LOOK_SLIDER_COUNT + 1;
const SCENE_LOOK_SCENE_PREV_ROW: usize = SCENE_LOOK_SLIDER_COUNT + 2;
const SCENE_LOOK_SCENE_NEXT_ROW: usize = SCENE_LOOK_SLIDER_COUNT + 3;
const SCENE_LOOK_ROW_COUNT: usize = SCENE_LOOK_SLIDER_COUNT + 4;

#[derive(Clone, Copy)]
struct SceneLookDebugLayout {
    panel_x: f32,
    panel_y: f32,
    panel_w: f32,
    rows_y0: f32,
    row_h: f32,
    row_gap: f32,
    label_w: f32,
    slider_w: f32,
    value_w: f32,
    scale: f32,
    row_count: usize,
}

impl SceneLookDebugLayout {
    fn compute(window_w: f32, window_h: f32, row_count: usize) -> Self {
        let scale = metrics::scene_scale(window_w, window_h);
        let row_h = (22.0 * scale).max(16.0);
        let row_gap = (3.0 * scale).max(2.0);
        let title_h = (24.0 * scale).max(16.0);
        let scene_h = (16.0 * scale).max(12.0);
        let pad = (8.0 * scale).max(5.0);
        let margin = (10.0 * scale).max(6.0);
        let panel_w = (320.0 * scale).min(window_w * 0.44);
        let panel_x = window_w - panel_w - margin;
        let panel_y = margin;
        let rows_y0 = panel_y + pad + title_h + scene_h + pad;
        let label_w = panel_w * 0.38;
        let slider_w = panel_w * 0.34;
        let value_w = (panel_w - label_w - slider_w - 12.0 * scale).max(36.0);
        Self {
            panel_x,
            panel_y,
            panel_w,
            rows_y0,
            row_h,
            row_gap,
            label_w,
            slider_w,
            value_w,
            scale,
            row_count,
        }
    }

    fn slider_track(&self, row: usize) -> (f32, f32, f32, f32) {
        let row_y = self.rows_y0 + row as f32 * (self.row_h + self.row_gap);
        let track_x = self.panel_x + self.label_w;
        let track_h = (5.0 * self.scale).max(3.0);
        let track_y = row_y + (self.row_h - track_h) * 0.5;
        (track_x, track_y, self.slider_w, track_h)
    }

    fn value_cell(&self, row: usize) -> (f32, f32, f32, f32) {
        let row_y = self.rows_y0 + row as f32 * (self.row_h + self.row_gap);
        let x = self.panel_x + self.label_w + self.slider_w + 4.0 * self.scale;
        (x, row_y, self.value_w, self.row_h)
    }

    fn action_row_rect(&self, row: usize) -> Option<(f32, f32, f32, f32)> {
        if !(SCENE_LOOK_SAVE_ROW..=SCENE_LOOK_SCENE_NEXT_ROW).contains(&row) {
            return None;
        }
        let row_y = self.rows_y0 + row as f32 * (self.row_h + self.row_gap);
        Some((self.panel_x + 4.0, row_y, self.panel_w - 8.0, self.row_h))
    }

    fn row_rect(&self, row: usize) -> Option<(f32, f32, f32, f32)> {
        if let Some(r) = self.action_row_rect(row) {
            return Some(r);
        }
        if row >= SCENE_LOOK_SLIDER_COUNT {
            return None;
        }
        let row_y = self.rows_y0 + row as f32 * (self.row_h + self.row_gap);
        Some((self.panel_x + 4.0, row_y, self.panel_w - 8.0, self.row_h))
    }
}

#[inline]
fn scene_look_point_in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx <= r.0 + r.2 && my >= r.1 && my <= r.1 + r.3
}

pub struct SceneLookDebugOverlay {
    cursor: usize,
    scene_index: usize,
    pub look: SceneLookTuning,
    editing: bool,
    edit_buffer: String,
    dragging_slider: Option<usize>,
    pointer: DebugPointerState,
}

pub enum SceneLookDebugResult {
    Stay,
    Close,
    Reset,
    Save,
}

impl SceneLookDebugOverlay {
    pub fn new(scene_index: usize, look: SceneLookTuning) -> Self {
        Self {
            cursor: 0,
            scene_index,
            look,
            editing: false,
            edit_buffer: String::new(),
            dragging_slider: None,
            pointer: DebugPointerState::default(),
        }
    }

    pub fn scene_key(&self) -> Option<&str> {
        let key = scene_look_tuning::overlay_scene_keys()[self.scene_index];
        if key == FALLBACK_SCENE_KEY {
            None
        } else {
            Some(key)
        }
    }

    pub fn scene_key_persist(&self) -> &str {
        scene_look_tuning::overlay_scene_keys()[self.scene_index]
    }

    fn row_count(&self) -> usize {
        SCENE_LOOK_ROW_COUNT
    }

    fn row_value(&self, row: usize) -> f32 {
        scene_look_tuning::scene_look_row_value(&self.look, row)
    }

    fn set_row_value(&mut self, row: usize, v: f32) {
        scene_look_tuning::scene_look_row_set(&mut self.look, row, v);
    }

    fn step_scene(&mut self, delta: i32, set: &SceneLookTuningSet) {
        let keys = scene_look_tuning::overlay_scene_keys();
        let n = keys.len() as i32;
        self.scene_index = ((self.scene_index as i32 + delta).rem_euclid(n)) as usize;
        self.look = set.resolve(self.scene_key());
        self.clear_edit();
    }

    fn apply_slider_mx(&mut self, row: usize, mx: f32, layout: &SceneLookDebugLayout) {
        let (tx, _, tw, _) = layout.slider_track(row);
        let (_, min, max, _) = SCENE_LOOK_SLIDER_META[row];
        let t = ((mx - tx) / tw.max(1e-6)).clamp(0.0, 1.0);
        self.set_row_value(row, min + t * (max - min));
    }

    fn adjust_row(&mut self, dir: f32) {
        if self.editing || self.cursor >= SCENE_LOOK_SLIDER_COUNT {
            return;
        }
        let row = self.cursor;
        let (_, _, _, step) = SCENE_LOOK_SLIDER_META[row];
        let v = self.row_value(row) + dir * step;
        self.set_row_value(row, v);
    }

    fn clear_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    fn begin_editing(&mut self) {
        let row = self.cursor;
        let v = self.row_value(row);
        self.edit_buffer = if scene_look_tuning::scene_look_row_is_hue(row) {
            format!("{}", (v * 360.0).round() as i32)
        } else if scene_look_tuning::scene_look_row_is_saturation(row) {
            format!("{:.0}", v * 100.0)
        } else {
            let mut s = format!("{:.6}", v);
            while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
                s.pop();
            }
            s
        };
        self.editing = true;
    }

    fn commit_edit(&mut self) {
        let row = self.cursor.min(self.row_count().saturating_sub(1));
        let t = self.edit_buffer.trim();
        let parsed = if scene_look_tuning::scene_look_row_is_hue(row) {
            t.parse::<f32>()
                .ok()
                .map(|deg| (deg / 360.0).fract())
                .or_else(|| t.parse::<f32>().ok().map(|h| h.fract()))
        } else if scene_look_tuning::scene_look_row_is_saturation(row) {
            t.trim_end_matches('%')
                .parse::<f32>()
                .ok()
                .map(|pct| (pct / 100.0).clamp(0.0, 1.0))
        } else {
            t.parse::<f32>().ok()
        };
        if let Some(v) = parsed {
            self.set_row_value(row, v);
        }
        self.clear_edit();
    }

    fn push_edit_char(&mut self, c: char) {
        if self.edit_buffer.len() >= 32 {
            return;
        }
        if c == '-' && !self.edit_buffer.is_empty() {
            return;
        }
        if c == '.' && self.edit_buffer.contains('.') {
            return;
        }
        self.edit_buffer.push(c);
    }

    /// Copy tonemap + room look as a `SceneLookTuning` Rust snapshot.
    pub fn copy_to_clipboard(&self) {
        let look = self.look;
        let t = look.tonemap;
        let r = look.room;
        let text = format!(
            concat!(
                "// scene: {}\n",
                "SceneLookTuning {{\n",
                "    tonemap: TonemapTuning {{\n",
                "        exposure: {:.6},\n",
                "        vhs_chromatic: {:.6},\n",
                "        vhs_scanline: {:.6},\n",
                "        vhs_grain: {:.6},\n",
                "        vhs_vignette: {:.6},\n",
                "    }},\n",
                "    room_gltf_height_scale: {:.6},\n",
                "    room: RoomEnvLightingTune {{\n",
                "        gltf_light_intensity_scale: {:.6},\n",
                "        linear_exposure: {:.6},\n",
                "        ambient_scale: {:.6},\n",
                "        lit_mesh_gltf_punctual_scale: {:.6},\n",
                "        gltf_emissive_scale: {:.6},\n",
                "        candle_light_color_mul: [{:.6}, {:.6}, {:.6}],\n",
                "        lantern_light_color_mul: [{:.6}, {:.6}, {:.6}],\n",
                "        ..RoomEnvLightingTune::SOURCE_DEFAULTS\n",
                "    }},\n",
                "}}\n",
            ),
            self.scene_key_persist(),
            t.exposure,
            t.vhs_chromatic,
            t.vhs_scanline,
            t.vhs_grain,
            t.vhs_vignette,
            look.room_gltf_height_scale,
            r.gltf_light_intensity_scale,
            r.linear_exposure,
            r.ambient_scale,
            r.lit_mesh_gltf_punctual_scale,
            r.gltf_emissive_scale,
            r.candle_light_color_mul[0],
            r.candle_light_color_mul[1],
            r.candle_light_color_mul[2],
            r.lantern_light_color_mul[0],
            r.lantern_light_color_mul[1],
            r.lantern_light_color_mul[2],
        );
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Err(e) = cb.set_text(&text) {
                    log::error!("Clipboard write failed: {e}");
                } else {
                    log::info!(
                        "Scene look snapshot copied (scene: {})",
                        self.scene_key_persist()
                    );
                }
            }
            Err(e) => log::error!("Could not open clipboard: {e}"),
        }
    }

    /// Keyboard while overlay is open. Returns `true` if the key was consumed
    /// (caller should skip the normal gameplay key dispatch).
    pub fn feed_key_event(&mut self, scancode: Option<Scancode>, ctrl: bool) -> bool {
        let Some(code) = scancode else {
            return false;
        };

        if ctrl && matches!(code, Scancode::C) && !self.editing {
            self.copy_to_clipboard();
            return true;
        }

        if !self.editing {
            return false;
        }

        match code {
            Scancode::Backspace => {
                let _ = self.edit_buffer.pop();
                true
            }
            Scancode::Escape => {
                self.clear_edit();
                true
            }
            Scancode::Return | Scancode::KpEnter => {
                self.commit_edit();
                true
            }
            Scancode::_0 | Scancode::Kp0 => {
                self.push_edit_char('0');
                true
            }
            Scancode::_1 | Scancode::Kp1 => {
                self.push_edit_char('1');
                true
            }
            Scancode::_2 | Scancode::Kp2 => {
                self.push_edit_char('2');
                true
            }
            Scancode::_3 | Scancode::Kp3 => {
                self.push_edit_char('3');
                true
            }
            Scancode::_4 | Scancode::Kp4 => {
                self.push_edit_char('4');
                true
            }
            Scancode::_5 | Scancode::Kp5 => {
                self.push_edit_char('5');
                true
            }
            Scancode::_6 | Scancode::Kp6 => {
                self.push_edit_char('6');
                true
            }
            Scancode::_7 | Scancode::Kp7 => {
                self.push_edit_char('7');
                true
            }
            Scancode::_8 | Scancode::Kp8 => {
                self.push_edit_char('8');
                true
            }
            Scancode::_9 | Scancode::Kp9 => {
                self.push_edit_char('9');
                true
            }
            Scancode::Period | Scancode::KpPeriod => {
                self.push_edit_char('.');
                true
            }
            Scancode::Minus | Scancode::KpMinus => {
                self.push_edit_char('-');
                true
            }
            _ => false,
        }
    }

    /// `mouse`: `(x, y, clicked_this_frame, left_button_held)`.
    pub fn update(
        &mut self,
        actions: &[UiAction],
        mouse: Option<(f32, f32, bool, bool)>,
        window_w: f32,
        window_h: f32,
        set: &SceneLookTuningSet,
    ) -> SceneLookDebugResult {
        let layout = SceneLookDebugLayout::compute(window_w, window_h, self.row_count());
        let slider_rows = SCENE_LOOK_SLIDER_COUNT;
        self.pointer.sync_held(mouse);
        self.pointer.clear_hover();

        if let Some((mx, my, clicked, held)) = mouse {
            for i in 0..self.row_count() {
                if let Some(rect) = layout.row_rect(i)
                    && scene_look_point_in_rect(mx, my, rect) {
                        self.pointer.hover_row = Some(i);
                        if self.dragging_slider.is_none() {
                            self.cursor = i;
                        }
                        break;
                    }
            }

            if let Some(di) = self.dragging_slider
                && held
                && di < slider_rows
            {
                self.apply_slider_mx(di, mx, &layout);
            }

            if (clicked || held) && self.dragging_slider.is_none() {
                for i in 0..slider_rows {
                    let track = layout.slider_track(i);
                    if scene_look_point_in_rect(mx, my, track) {
                        self.cursor = i;
                        self.clear_edit();
                        self.apply_slider_mx(i, mx, &layout);
                        if held {
                            self.dragging_slider = Some(i);
                        }
                        break;
                    }
                }
            }

            if clicked && self.dragging_slider.is_none() {
                for i in 0..slider_rows {
                    let cell = layout.value_cell(i);
                    if scene_look_point_in_rect(mx, my, cell) {
                        self.cursor = i;
                        self.begin_editing();
                        break;
                    }
                }
                for row in SCENE_LOOK_SAVE_ROW..=SCENE_LOOK_SCENE_NEXT_ROW {
                    let Some(rect) = layout.action_row_rect(row) else {
                        continue;
                    };
                    if scene_look_point_in_rect(mx, my, rect) {
                        self.cursor = row;
                        self.clear_edit();
                        return match row {
                            SCENE_LOOK_SAVE_ROW => SceneLookDebugResult::Save,
                            SCENE_LOOK_RESET_ROW => SceneLookDebugResult::Reset,
                            SCENE_LOOK_SCENE_PREV_ROW => {
                                self.step_scene(-1, set);
                                SceneLookDebugResult::Stay
                            }
                            _ => {
                                self.step_scene(1, set);
                                SceneLookDebugResult::Stay
                            }
                        };
                    }
                }
            }
        }

        if let Some((_, _, _, held)) = mouse {
            if !held {
                self.dragging_slider = None;
            }
        } else {
            self.dragging_slider = None;
        }

        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % self.row_count();
                    self.clear_edit();
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + self.row_count() - 1) % self.row_count();
                    self.clear_edit();
                }
                UiAction::FocusNext
                | UiAction::FocusPrev
                | UiAction::NavigateHudNext
                | UiAction::NavigateHudPrev => {
                    if self.editing {
                        continue;
                    }
                    let dir = match a {
                        UiAction::FocusPrev | UiAction::NavigateHudPrev => -1.0,
                        _ => 1.0,
                    };
                    if self.cursor == SCENE_LOOK_SCENE_PREV_ROW {
                        self.step_scene(-1, set);
                    } else if self.cursor == SCENE_LOOK_SCENE_NEXT_ROW {
                        self.step_scene(1, set);
                    } else {
                        self.adjust_row(dir);
                    }
                }
                UiAction::Confirm | UiAction::CommitDiscard => {
                    if self.editing {
                        self.commit_edit();
                    } else if self.cursor == SCENE_LOOK_SAVE_ROW {
                        return SceneLookDebugResult::Save;
                    } else if self.cursor == SCENE_LOOK_RESET_ROW {
                        return SceneLookDebugResult::Reset;
                    } else if self.cursor == SCENE_LOOK_SCENE_PREV_ROW {
                        self.step_scene(-1, set);
                    } else if self.cursor == SCENE_LOOK_SCENE_NEXT_ROW {
                        self.step_scene(1, set);
                    } else if self.cursor < SCENE_LOOK_SLIDER_COUNT {
                        self.begin_editing();
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    if self.editing {
                        self.clear_edit();
                    } else {
                        return SceneLookDebugResult::Close;
                    }
                }
                _ => {}
            }
        }
        SceneLookDebugResult::Stay
    }

    fn format_row_display(row: usize, v: f32) -> String {
        let (_, _, _, step) = SCENE_LOOK_SLIDER_META[row];
        if scene_look_tuning::scene_look_row_is_hue(row) {
            return format!("{}°", (v * 360.0).round() as i32);
        }
        if scene_look_tuning::scene_look_row_is_saturation(row) {
            return format!("{:.0}%", v * 100.0);
        }
        match row {
            14 | 17 => format!("{v:.2}"),
            1 => format!("{v:.4}"),
            _ if step >= 0.01 => format!("{v:.2}"),
            _ if step >= 0.001 => format!("{v:.3}"),
            _ => format!("{v:.4}"),
        }
    }

    fn draw_hue_slider_track(
        track_x: f32,
        track_y: f32,
        tw: f32,
        th: f32,
        instances: &mut Vec<GpuInstance>,
    ) {
        const SEGMENTS: usize = 12;
        let seg_w = tw / SEGMENTS as f32;
        for i in 0..SEGMENTS {
            let h = i as f32 / SEGMENTS as f32;
            let preview = hue_wheel_preview_linear(h);
            instances.push(GpuInstance {
                rect: [track_x + seg_w * i as f32, track_y, seg_w + 0.5, th],
                color: [preview[0], preview[1], preview[2], 1.0],
                user: 0,
            });
        }
    }

    fn draw_action_row(
        &self,
        layout: &SceneLookDebugLayout,
        row: usize,
        label: &str,
        instances: &mut Vec<GpuInstance>,
        labels: &mut Vec<TextLabel>,
        variant: ButtonVariant,
    ) {
        let row_y = layout.rows_y0 + row as f32 * (layout.row_h + layout.row_gap);
        let visual = DebugRowVisual::for_row(row, self.cursor, &self.pointer);
        let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, variant);
        instances.push(GpuInstance {
            rect: [
                layout.panel_x + 4.0,
                row_y,
                layout.panel_w - 8.0,
                layout.row_h,
            ],
            color: bg,
            user: 0,
        });
        labels.push(TextLabel {
            rect: [layout.panel_x, row_y, layout.panel_w, layout.row_h],
            text: label.into(),
            color: tc,
            ..Default::default()
        });
    }

    pub fn draw(&self, window_w: f32, window_h: f32) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let layout = SceneLookDebugLayout::compute(window_w, window_h, self.row_count());
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        let pad = (8.0 * layout.scale).max(5.0);
        let title_h = (24.0 * layout.scale).max(16.0);
        let scene_h = (16.0 * layout.scale).max(12.0);
        let hint_h = (13.0 * layout.scale).max(9.0);
        let panel_h = pad
            + title_h
            + scene_h
            + pad
            + layout.row_count as f32 * (layout.row_h + layout.row_gap)
            + pad
            + hint_h * 2.0
            + pad * 2.0;

        let border = 2.0;
        instances.push(GpuInstance {
            rect: [
                layout.panel_x - border,
                layout.panel_y - border,
                layout.panel_w + border * 2.0,
                panel_h + border * 2.0,
            ],
            color: color::alpha(color::WALNUT_SOFT, 0.85),
            user: 0,
        });
        instances.push(GpuInstance {
            rect: [layout.panel_x, layout.panel_y, layout.panel_w, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.97),
            user: 0,
        });

        labels.push(TextLabel {
            rect: [
                layout.panel_x,
                layout.panel_y + pad,
                layout.panel_w,
                title_h,
            ],
            text: "Scene Look".into(),
            color: color::JADE,
            ..Default::default()
        });

        let scene_label = if self.scene_key_persist() == FALLBACK_SCENE_KEY {
            "scene: (default / no key)".to_string()
        } else {
            format!("scene: {}", self.scene_key_persist())
        };
        labels.push(TextLabel {
            rect: [
                layout.panel_x,
                layout.panel_y + pad + title_h,
                layout.panel_w,
                scene_h,
            ],
            text: scene_label,
            color: color::alpha(color::CHAMPAGNE, 0.75),
            ..Default::default()
        });

        for (i, (name, min, max, _)) in SCENE_LOOK_SLIDER_META
            .iter()
            .enumerate()
            .take(SCENE_LOOK_SLIDER_COUNT)
        {
            let row_y = layout.rows_y0 + i as f32 * (layout.row_h + layout.row_gap);
            let visual = DebugRowVisual::for_row(i, self.cursor, &self.pointer);
            let v = self.row_value(i);

            let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, ButtonVariant::Default);
            instances.push(GpuInstance {
                rect: [
                    layout.panel_x + 4.0,
                    row_y,
                    layout.panel_w - 8.0,
                    layout.row_h,
                ],
                color: bg,
                user: 0,
            });

            let swatch = (14.0 * layout.scale).max(10.0);
            let mut label_x = layout.panel_x + 6.0 * layout.scale;
            let mut label_w = layout.label_w - 4.0 * layout.scale;
            if let Some(rgb) = scene_look_tuning::scene_look_tint_swatch_rgb(&self.look, i) {
                let sw_y = row_y + (layout.row_h - swatch) * 0.5;
                instances.push(GpuInstance {
                    rect: [label_x, sw_y, swatch, swatch],
                    color: [rgb[0], rgb[1], rgb[2], 1.0],
                    user: 0,
                });
                instances.push(GpuInstance {
                    rect: [label_x - 1.0, sw_y - 1.0, swatch + 2.0, swatch + 2.0],
                    color: color::alpha(color::PARCHMENT, if visual.highlighted { 0.55 } else { 0.28 }),
                    user: 0,
                });
                label_x += swatch + 4.0 * layout.scale;
                label_w -= swatch + 4.0 * layout.scale;
            }
            labels.push(TextLabel {
                rect: [label_x, row_y, label_w, layout.row_h],
                text: (*name).into(),
                color: tc,
                ..Default::default()
            });

            let (track_x, track_y, tw, th) = layout.slider_track(i);
            let t = ((v - *min) / (*max - *min).max(1e-8)).clamp(0.0, 1.0);
            let fill_w = tw * t;
            if scene_look_tuning::scene_look_row_is_hue(i) {
                Self::draw_hue_slider_track(track_x, track_y, tw, th, &mut instances);
            } else {
                instances.push(GpuInstance {
                    rect: [track_x, track_y, tw, th],
                    color: color::WALNUT_INK,
                    user: 0,
                });
                let fill_color = scene_look_tuning::scene_look_tint_swatch_rgb(&self.look, i)
                    .map(|[r, g, b]| {
                        if visual.highlighted {
                            [r, g, b, 0.95]
                        } else {
                            [r * 0.85, g * 0.85, b * 0.85, 0.75]
                        }
                    })
                    .unwrap_or_else(|| {
                        if visual.highlighted {
                            color::JADE
                        } else {
                            color::alpha(color::JADE, 0.7)
                        }
                    });
                instances.push(GpuInstance {
                    rect: [track_x, track_y, fill_w, th],
                    color: fill_color,
                    user: 0,
                });
            }
            let knob_size = th * 2.5;
            let knob_x = track_x + fill_w - knob_size * 0.5;
            let knob_y = track_y + (th - knob_size) * 0.5;
            instances.push(GpuInstance {
                rect: [knob_x, knob_y, knob_size, knob_size],
                color: if visual.highlighted {
                    color::PARCHMENT
                } else {
                    color::alpha(color::STONE, 0.95)
                },
                user: 0,
            });

            let (vx, vy, vw, vh) = layout.value_cell(i);
            let value_text = if self.editing && i == self.cursor {
                format!(
                    "{}{}",
                    self.edit_buffer,
                    if visual.highlighted { "\u{258c}" } else { "" }
                )
            } else {
                Self::format_row_display(i, v)
            };
            instances.push(GpuInstance {
                rect: [vx, vy + vh * 0.15, vw, vh * 0.7],
                color: if self.editing && i == self.cursor {
                    color::alpha(color::WALNUT_INK, 0.95)
                } else {
                    color::alpha(color::WALNUT_INK, 0.75)
                },
                user: 0,
            });
            labels.push(TextLabel {
                rect: [vx + 2.0 * layout.scale, vy, vw - 4.0 * layout.scale, vh],
                text: value_text,
                color: [tc[0] * 0.92, tc[1] * 0.92, tc[2] * 0.92, 1.0],
                font_px: Some(crate::render::theme::typography::tier_at_most(
                    layout.row_h * 0.48,
                    window_h,
                )),
                ..Default::default()
            });
        }

        self.draw_action_row(
            &layout,
            SCENE_LOOK_SAVE_ROW,
            "Save for this scene",
            &mut instances,
            &mut labels,
            ButtonVariant::Primary,
        );
        self.draw_action_row(
            &layout,
            SCENE_LOOK_RESET_ROW,
            "Reset scene to default",
            &mut instances,
            &mut labels,
            ButtonVariant::Danger,
        );
        self.draw_action_row(
            &layout,
            SCENE_LOOK_SCENE_PREV_ROW,
            "\u{2190} Previous scene",
            &mut instances,
            &mut labels,
            ButtonVariant::Default,
        );
        self.draw_action_row(
            &layout,
            SCENE_LOOK_SCENE_NEXT_ROW,
            "Next scene \u{2192}",
            &mut instances,
            &mut labels,
            ButtonVariant::Default,
        );

        let hint_y =
            layout.rows_y0 + layout.row_count as f32 * (layout.row_h + layout.row_gap) + pad;
        labels.push(TextLabel {
            rect: [layout.panel_x, hint_y, layout.panel_w, hint_h],
            text: "Mouse: drag slider / click value to type".into(),
            color: color::alpha(color::STONE, 0.75),
            ..Default::default()
        });
        labels.push(TextLabel {
            rect: [layout.panel_x, hint_y + hint_h, layout.panel_w, hint_h],
            text: "\u{2191}\u{2193} row  \u{2190}\u{2192} nudge/switch scene  Enter  Esc  Ctrl+C"
                .into(),
            color: color::alpha(color::STONE, 0.75),
            ..Default::default()
        });

        (instances, labels)
    }
}

// ── Pick-blind hallway distortion debug (vertex warp tuning) ─────────────

const HALL_DIST_DEBUG_ROW_COUNT: usize = 15;

const HALL_DIST_DEBUG_ROW_META: &[(&str, f32, f32, f32)] = &[
    ("Blind (0 Auto 1S 2B 3Boss)", 0.0, 3.0, 0.25),
    ("Run seed lo (u32)", 0.0, 4_294_967_295.0, 1.0),
    ("Run seed hi (u32)", 0.0, 4_294_967_295.0, 1.0),
    ("Global intensity ×", 0.0, 10.0, 0.02),
    ("Breathe amp ×", 0.0, 10.0, 0.05),
    ("Ripple amp ×", 0.0, 10.0, 0.05),
    ("Ripple waves ×", 0.0, 3.0, 0.05),
    ("Ripple travel ×", 0.0, 3.0, 0.05),
    ("Wall barrel bow ×", 0.0, 10.0, 0.05),
    ("Wall tint (0 Auto 1–8)", 0.0, 8.0, 1.0),
    ("Ceiling squeeze ×", 0.0, 10.0, 0.05),
    ("Depth stretch × (×u)", 0.0, 10.0, 0.05),
    ("Twist × (walls)", 0.0, 10.0, 0.05),
    ("Mask pulse ×", 0.0, 10.0, 0.05),
    ("Phase drift ×", 0.0, 10.0, 0.05),
];

#[derive(Clone, Copy)]
struct HallDistDebugLayout {
    panel_x: f32,
    panel_y: f32,
    panel_w: f32,
    rows_y0: f32,
    row_h: f32,
    row_gap: f32,
    label_w: f32,
    slider_w: f32,
    value_w: f32,
    scale: f32,
}

impl HallDistDebugLayout {
    fn compute(window_w: f32, window_h: f32, row_count: usize) -> Self {
        let scale = metrics::scene_scale(window_w, window_h);
        let row_h = (22.0 * scale).max(16.0);
        let row_gap = (3.0 * scale).max(2.0);
        let title_h = (24.0 * scale).max(16.0);
        let subtitle_h = (18.0 * scale).max(13.0);
        let pad = (8.0 * scale).max(5.0);
        let margin = (10.0 * scale).max(6.0);
        let panel_w = (420.0 * scale).min(window_w * 0.56);
        let panel_x = margin;
        let panel_y = margin;
        let rows_y0 = panel_y + pad + title_h + subtitle_h + pad;
        let label_w = panel_w * 0.48;
        let slider_w = panel_w * 0.26;
        let value_w = (panel_w - label_w - slider_w - 12.0 * scale).max(88.0);
        let _ = row_count;
        Self {
            panel_x,
            panel_y,
            panel_w,
            rows_y0,
            row_h,
            row_gap,
            label_w,
            slider_w,
            value_w,
            scale,
        }
    }

    fn slider_track(&self, row: usize) -> (f32, f32, f32, f32) {
        let row_y = self.rows_y0 + row as f32 * (self.row_h + self.row_gap);
        let track_x = self.panel_x + self.label_w;
        let track_h = (5.0 * self.scale).max(3.0);
        let track_y = row_y + (self.row_h - track_h) * 0.5;
        (track_x, track_y, self.slider_w, track_h)
    }

    fn value_cell(&self, row: usize) -> (f32, f32, f32, f32) {
        let row_y = self.rows_y0 + row as f32 * (self.row_h + self.row_gap);
        let x = self.panel_x + self.label_w + self.slider_w + 4.0 * self.scale;
        (x, row_y, self.value_w, self.row_h)
    }

    fn row_rect(&self, row: usize) -> (f32, f32, f32, f32) {
        let row_y = self.rows_y0 + row as f32 * (self.row_h + self.row_gap);
        (self.panel_x + 4.0, row_y, self.panel_w - 8.0, self.row_h)
    }
}

#[inline]
fn hall_dist_point_in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx <= r.0 + r.2 && my >= r.1 && my <= r.1 + r.3
}

pub struct HallwayDistortionDebugOverlay {
    cursor: usize,
    chamber_row: u8,
    seed_lo: f32,
    seed_hi: f32,
    run_number: f32,
    wing: f32,
    global_mul: f32,
    breathe_mul: f32,
    ceiling_mul: f32,
    stretch_mul: f32,
    twist_mul: f32,
    pulse_mul: f32,
    drift_mul: f32,
    ripple_mul: f32,
    balloon_mul: f32,
    wall_tint: f32,
    ripple_waves_mul: f32,
    ripple_travel_mul: f32,
    editing: bool,
    edit_buffer: String,
    dragging_slider: Option<usize>,
    pointer: DebugPointerState,
}

impl HallwayDistortionDebugOverlay {
    pub fn from_run(run_seed: u64, run_number: u32, wing: u32) -> Self {
        Self {
            cursor: 0,
            chamber_row: 0,
            seed_lo: (run_seed as u32) as f32,
            seed_hi: (run_seed >> 32) as f32,
            run_number: run_number.max(1) as f32,
            wing: wing.max(1) as f32,
            global_mul: 1.0,
            breathe_mul: 1.0,
            ceiling_mul: 1.0,
            stretch_mul: 1.0,
            twist_mul: 1.0,
            pulse_mul: 1.0,
            drift_mul: 1.0,
            ripple_mul: 1.0,
            balloon_mul: 1.0,
            wall_tint: 0.0,
            ripple_waves_mul: 1.0,
            ripple_travel_mul: 1.0,
            editing: false,
            edit_buffer: String::new(),
            dragging_slider: None,
            pointer: DebugPointerState::default(),
        }
    }

    pub fn to_snapshot(&self) -> HallwayDistortionDebugSnapshot {
        let lo = self.seed_lo.round().clamp(0.0, u32::MAX as f32) as u32;
        let hi = self.seed_hi.round().clamp(0.0, u32::MAX as f32) as u32;
        HallwayDistortionDebugSnapshot {
            chamber_mode: self.chamber_row.min(3),
            run_seed: u64::from(lo) | (u64::from(hi) << 32),
            run_number: self.run_number.round().clamp(1.0, 999.0) as u32,
            wing: self.wing.round().clamp(1.0, 99.0) as u32,
            global_mul: self.global_mul,
            breathe_mul: self.breathe_mul,
            ceiling_mul: self.ceiling_mul,
            stretch_mul: self.stretch_mul,
            twist_mul: self.twist_mul,
            pulse_mul: self.pulse_mul,
            drift_mul: self.drift_mul,
            ripple_mul: self.ripple_mul,
            balloon_mul: self.balloon_mul,
            wall_tint: self.wall_tint.round().clamp(0.0, 6.0) as u8,
            ripple_waves_mul: self.ripple_waves_mul,
            ripple_travel_mul: self.ripple_travel_mul,
        }
    }

    fn row_count(&self) -> usize {
        HALL_DIST_DEBUG_ROW_COUNT
    }

    fn row_value(&self, row: usize) -> f32 {
        match row {
            0 => self.chamber_row as f32,
            1 => self.seed_lo,
            2 => self.seed_hi,
            3 => self.global_mul,
            4 => self.breathe_mul,
            5 => self.ripple_mul,
            6 => self.ripple_waves_mul,
            7 => self.ripple_travel_mul,
            8 => self.balloon_mul,
            9 => self.wall_tint,
            10 => self.ceiling_mul,
            11 => self.stretch_mul,
            12 => self.twist_mul,
            13 => self.pulse_mul,
            14 => self.drift_mul,
            _ => 0.0,
        }
    }

    fn set_row_value(&mut self, row: usize, v: f32) {
        let (_, lo, hi, _) = HALL_DIST_DEBUG_ROW_META[row];
        let v = v.clamp(lo, hi);
        match row {
            0 => self.chamber_row = (v.round() as i32).clamp(0, 3) as u8,
            1 => self.seed_lo = v,
            2 => self.seed_hi = v,
            3 => self.global_mul = v,
            4 => self.breathe_mul = v,
            5 => self.ripple_mul = v,
            6 => self.ripple_waves_mul = v,
            7 => self.ripple_travel_mul = v,
            8 => self.balloon_mul = v,
            9 => self.wall_tint = v.round(),
            10 => self.ceiling_mul = v,
            11 => self.stretch_mul = v,
            12 => self.twist_mul = v,
            13 => self.pulse_mul = v,
            14 => self.drift_mul = v,
            _ => {}
        }
    }

    fn apply_slider_mx(&mut self, row: usize, mx: f32, layout: &HallDistDebugLayout) {
        let (tx, _, tw, _) = layout.slider_track(row);
        let (_, min, max, _) = HALL_DIST_DEBUG_ROW_META[row];
        let t = ((mx - tx) / tw.max(1e-6)).clamp(0.0, 1.0);
        self.set_row_value(row, min + t * (max - min));
    }

    fn adjust_row(&mut self, dir: f32) {
        if self.editing {
            return;
        }
        let row = self.cursor.min(self.row_count().saturating_sub(1));
        let (_, _, _, step) = HALL_DIST_DEBUG_ROW_META[row];
        let v = self.row_value(row) + dir * step;
        self.set_row_value(row, v);
    }

    fn clear_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    fn begin_editing(&mut self) {
        let v = self.row_value(self.cursor);
        let mut s = if self.cursor == 0 {
            format!("{}", self.chamber_row)
        } else if self.cursor == 9 {
            format!("{}", self.wall_tint.round() as u8)
        } else {
            format!("{:.6}", v)
        };
        while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
            s.pop();
        }
        self.edit_buffer = s;
        self.editing = true;
    }

    fn commit_edit(&mut self) {
        let row = self.cursor.min(self.row_count().saturating_sub(1));
        let t = self.edit_buffer.trim();
        if let Ok(v) = t.parse::<f32>() {
            self.set_row_value(row, v);
        }
        self.clear_edit();
    }

    fn push_edit_char(&mut self, c: char) {
        if self.edit_buffer.len() >= 32 {
            return;
        }
        if c == '-' && !self.edit_buffer.is_empty() {
            return;
        }
        if c == '.' && self.edit_buffer.contains('.') {
            return;
        }
        self.edit_buffer.push(c);
    }

    pub fn feed_key_event(&mut self, scancode: Option<Scancode>, ctrl: bool) -> bool {
        let Some(code) = scancode else {
            return false;
        };

        if ctrl && matches!(code, Scancode::C) && !self.editing {
            let s = self.to_snapshot();
            let text = format!(
                "HallwayDistortionDebugSnapshot {{ chamber_mode: {}, run_seed: {:#018x}, run_number: {}, wing: {}, global_mul: {:.4}, breathe_mul: {:.4}, ripple_mul: {:.4}, ceiling_mul: {:.4}, stretch_mul: {:.4}, twist_mul: {:.4}, pulse_mul: {:.4}, drift_mul: {:.4}, balloon_mul: {:.4}, wall_tint: {}, ripple_waves_mul: {:.4}, ripple_travel_mul: {:.4} }}",
                s.chamber_mode,
                s.run_seed,
                s.run_number,
                s.wing,
                s.global_mul,
                s.breathe_mul,
                s.ripple_mul,
                s.ceiling_mul,
                s.stretch_mul,
                s.twist_mul,
                s.pulse_mul,
                s.drift_mul,
                s.balloon_mul,
                s.wall_tint,
                s.ripple_waves_mul,
                s.ripple_travel_mul,
            );
            match arboard::Clipboard::new() {
                Ok(mut cb) => {
                    if let Err(e) = cb.set_text(&text) {
                        log::error!("Clipboard write failed: {e}");
                    } else {
                        log::info!("Hallway vertex warp snapshot copied to clipboard");
                    }
                }
                Err(e) => log::error!("Could not open clipboard: {e}"),
            }
            return true;
        }

        if !self.editing {
            return false;
        }

        match code {
            Scancode::Backspace => {
                let _ = self.edit_buffer.pop();
                true
            }
            Scancode::Escape => {
                self.clear_edit();
                true
            }
            Scancode::Return | Scancode::KpEnter => {
                self.commit_edit();
                true
            }
            Scancode::_0 | Scancode::Kp0 => {
                self.push_edit_char('0');
                true
            }
            Scancode::_1 | Scancode::Kp1 => {
                self.push_edit_char('1');
                true
            }
            Scancode::_2 | Scancode::Kp2 => {
                self.push_edit_char('2');
                true
            }
            Scancode::_3 | Scancode::Kp3 => {
                self.push_edit_char('3');
                true
            }
            Scancode::_4 | Scancode::Kp4 => {
                self.push_edit_char('4');
                true
            }
            Scancode::_5 | Scancode::Kp5 => {
                self.push_edit_char('5');
                true
            }
            Scancode::_6 | Scancode::Kp6 => {
                self.push_edit_char('6');
                true
            }
            Scancode::_7 | Scancode::Kp7 => {
                self.push_edit_char('7');
                true
            }
            Scancode::_8 | Scancode::Kp8 => {
                self.push_edit_char('8');
                true
            }
            Scancode::_9 | Scancode::Kp9 => {
                self.push_edit_char('9');
                true
            }
            Scancode::Period | Scancode::KpPeriod => {
                self.push_edit_char('.');
                true
            }
            Scancode::Minus | Scancode::KpMinus => {
                self.push_edit_char('-');
                true
            }
            _ => false,
        }
    }

    /// Returns `true` when the overlay should close.
    pub fn update(
        &mut self,
        actions: &[UiAction],
        mouse: Option<(f32, f32, bool, bool)>,
        window_w: f32,
        window_h: f32,
    ) -> bool {
        let layout = HallDistDebugLayout::compute(window_w, window_h, self.row_count());
        let n = self.row_count();
        self.pointer.sync_held(mouse);
        self.pointer.clear_hover();

        if let Some((mx, my, clicked, held)) = mouse {
            for i in 0..n {
                if hall_dist_point_in_rect(mx, my, layout.row_rect(i)) {
                    self.pointer.hover_row = Some(i);
                    if self.dragging_slider.is_none() {
                        self.cursor = i;
                    }
                    break;
                }
            }

            if let Some(di) = self.dragging_slider
                && held
            {
                self.apply_slider_mx(di, mx, &layout);
            }

            if (clicked || held) && self.dragging_slider.is_none() {
                for i in 0..n {
                    let track = layout.slider_track(i);
                    if hall_dist_point_in_rect(mx, my, track) {
                        self.cursor = i;
                        self.clear_edit();
                        self.apply_slider_mx(i, mx, &layout);
                        if held {
                            self.dragging_slider = Some(i);
                        }
                        break;
                    }
                }
            }

            if clicked && self.dragging_slider.is_none() {
                for i in 0..n {
                    let cell = layout.value_cell(i);
                    if hall_dist_point_in_rect(mx, my, cell) {
                        self.cursor = i;
                        self.begin_editing();
                        break;
                    }
                }
            }
        }

        if let Some((_, _, _, held)) = mouse {
            if !held {
                self.dragging_slider = None;
            }
        } else {
            self.dragging_slider = None;
        }

        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % self.row_count();
                    self.clear_edit();
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + self.row_count() - 1) % self.row_count();
                    self.clear_edit();
                }
                UiAction::FocusNext
                | UiAction::FocusPrev
                | UiAction::NavigateHudNext
                | UiAction::NavigateHudPrev => {
                    if self.editing {
                        continue;
                    }
                    let dir = match a {
                        UiAction::FocusPrev | UiAction::NavigateHudPrev => -1.0,
                        _ => 1.0,
                    };
                    self.adjust_row(dir);
                }
                UiAction::Confirm | UiAction::CommitDiscard => {
                    if self.editing {
                        self.commit_edit();
                    } else {
                        self.begin_editing();
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    if self.editing {
                        self.clear_edit();
                    } else {
                        return true;
                    }
                }
                _ => {}
            }
        }

        false
    }

    fn preview_blind(&self) -> ChamberKind {
        match self.chamber_row {
            0 => ChamberKind::Big,
            1 => ChamberKind::Small,
            2 => ChamberKind::Big,
            _ => ChamberKind::Ordeal,
        }
    }

    fn preview_distortion(&self, window_h: f32, env_height_scale: f32) -> HallwayDistortion {
        let mut d = self.to_snapshot().resolve(self.preview_blind());
        hallway_distortion_apply_glb_depth_extent(&mut d, window_h, env_height_scale);
        d
    }

    fn trim_f32(v: f32) -> String {
        let mut s = format!("{:.4}", v);
        while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
            s.pop();
        }
        s
    }

    fn format_row_display(&self, row: usize, preview: &HallwayDistortion) -> String {
        if self.editing && self.cursor == row {
            return self.edit_buffer.clone();
        }
        if row == 0 {
            return match self.chamber_row {
                0 => "0 Auto → Big".into(),
                1 => "1 Small".into(),
                2 => "2 Big".into(),
                _ => "3 Boss".into(),
            };
        }
        if row == 9 {
            let idx = self.wall_tint.round() as u8;
            let names = [
                "Auto", "Blue", "Gold", "Green", "Red", "Purple", "Orange", "Pink", "Brown",
            ];
            let label = names.get(idx as usize).copied().unwrap_or("?");
            if self.editing && self.cursor == row {
                return self.edit_buffer.clone();
            }
            let rgb = if idx == 0 {
                [preview.bow[0], preview.bow[1], preview.bow[2]]
            } else {
                crate::render::hallway_glb::hallway_wall_tint_by_index(idx as usize - 1)
            };
            return format!(
                "{idx} {label} ({},{},{})",
                Self::trim_f32(rgb[0]),
                Self::trim_f32(rgb[1]),
                Self::trim_f32(rgb[2]),
            );
        }
        let v = self.row_value(row);
        let mul = Self::trim_f32(v);
        let eff = match row {
            3 => Self::trim_f32(preview.flags[2]),
            4 => Self::trim_f32(preview.breathe[0]),
            5 => Self::trim_f32(preview.ripple[0]),
            6 => Self::trim_f32(preview.ripple[1]),
            7 => Self::trim_f32(preview.ripple[3]),
            8 => Self::trim_f32(preview.bow[3]),
            10 => Self::trim_f32(preview.ceiling[0]),
            11 => Self::trim_f32(preview.stretch[0]),
            12 => Self::trim_f32(preview.twist[0]),
            13 => Self::trim_f32(preview.time_pulse[3]),
            14 => Self::trim_f32(preview.time_pulse[1]),
            _ => return mul,
        };
        format!("{mul} ({eff})")
    }

    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        env_height_scale: f32,
    ) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let layout = HallDistDebugLayout::compute(window_w, window_h, self.row_count());
        let scale = layout.scale;
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        let preview = self.preview_distortion(window_h, env_height_scale);
        let title_h = (24.0 * scale).max(16.0);
        let subtitle_h = (18.0 * scale).max(13.0);
        let pad = (8.0 * scale).max(5.0);
        let row_gap = layout.row_gap;
        let hint_h = (18.0 * scale).max(13.0);
        let panel_h = layout.panel_y
            + pad
            + title_h
            + subtitle_h
            + pad
            + self.row_count() as f32 * (layout.row_h + row_gap)
            + hint_h * 2.0
            + row_gap * 2.0
            - layout.panel_y;

        let border = 2.0;
        let px = layout.panel_x;
        let py = layout.panel_y;
        let pw = layout.panel_w;
        instances.push(GpuInstance {
            rect: [
                px - border,
                py - border,
                pw + border * 2.0,
                panel_h + border * 2.0,
            ],
            color: color::alpha(color::ANTIQUE, 0.85),
            user: 0,
        });
        instances.push(GpuInstance {
            rect: [px, py, pw, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.92),
            user: 0,
        });

        labels.push(TextLabel {
            rect: [px, py + row_gap, pw, title_h],
            text: "Hallway hall FX".into(),
            color: color::PARCHMENT,
            ..Default::default()
        });
        labels.push(TextLabel {
            rect: [px, py + row_gap + title_h, pw, subtitle_h],
            text:
                "breathe · ripple · wall bow (bow.w × |side_c|) · tint (bow.rgb) · squeeze/stretch"
                    .into(),
            color: color::alpha(color::STONE, 0.88),
            ..Default::default()
        });

        for (i, (name, _, _, _)) in HALL_DIST_DEBUG_ROW_META
            .iter()
            .enumerate()
            .take(self.row_count())
        {
            let row_y = layout.rows_y0 + i as f32 * (layout.row_h + row_gap);
            let visual = DebugRowVisual::for_row(i, self.cursor, &self.pointer);
            let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, ButtonVariant::Default);
            instances.push(GpuInstance {
                rect: [px + 4.0, row_y, pw - 8.0, layout.row_h],
                color: bg,
                user: 0,
            });
            labels.push(TextLabel {
                rect: [px + 6.0 * scale, row_y, layout.label_w, layout.row_h],
                text: (*name).into(),
                color: tc,
                ..Default::default()
            });

            let (track_x, track_y, tw, th) = layout.slider_track(i);
            instances.push(GpuInstance {
                rect: [track_x, track_y, tw, th],
                color: color::WALNUT_INK,
                user: 0,
            });

            let (_, min, max, _) = HALL_DIST_DEBUG_ROW_META[i];
            let v = self.row_value(i);
            let t = ((v - min) / (max - min)).clamp(0.0, 1.0);
            let fill_w = tw * t;
            let fill_color = if visual.highlighted {
                color::AMBER
            } else {
                color::alpha(color::ANTIQUE, 0.95)
            };
            instances.push(GpuInstance {
                rect: [track_x, track_y, fill_w, th],
                color: fill_color,
                user: 0,
            });

            let knob_size = th * 2.5;
            let knob_x = track_x + fill_w - knob_size * 0.5;
            let knob_y = track_y + (th - knob_size) * 0.5;
            instances.push(GpuInstance {
                rect: [knob_x, knob_y, knob_size, knob_size],
                color: if visual.highlighted {
                    color::PARCHMENT
                } else {
                    color::alpha(color::STONE, 0.9)
                },
                user: 0,
            });

            let (vx, vy, vw, vh) = layout.value_cell(i);
            labels.push(TextLabel {
                rect: [vx, vy, vw, vh],
                text: self.format_row_display(i, &preview),
                color: tc,
                ..Default::default()
            });
        }

        let hint_y = layout.rows_y0 + self.row_count() as f32 * (layout.row_h + row_gap) + row_gap;
        labels.push(TextLabel {
            rect: [px, hint_y, pw, hint_h],
            text: "\u{2191}/\u{2193} row   \u{2190}/\u{2192} adjust   \u{23ce} type value   Esc close   Ctrl+C copy"
                .into(),
            color: color::alpha(color::STONE, 0.85),
            ..Default::default()
        });
        labels.push(TextLabel {
            rect: [px, hint_y + hint_h, pw, hint_h],
            text: "Value (eff): slider × resolved GPU amp for preview blind".into(),
            color: color::alpha(color::STONE, 0.75),
            ..Default::default()
        });

        (instances, labels)
    }
}
