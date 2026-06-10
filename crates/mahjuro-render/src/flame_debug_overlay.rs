//! Debug slider panel for [`super::flame_tuning::FlameTuning`] — shop / gameplay candle flames.

use sdl3::keyboard::Scancode;

use crate::debug_overlay_ui::{self, DebugPointerState, DebugRowVisual};
use crate::flame_tuning::{FLAME_DEBUG_ROW_META, FLAME_DEBUG_SLIDER_COUNT, FlameTuning};
use crate::theme::{ButtonVariant, color, metrics, typography};
use crate::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use mahjuro_types::UiAction;

const FLAME_MID_ACTION_COUNT: usize = 2;
const FLAME_TRIGGER_GUST_ROW: usize = FLAME_DEBUG_SLIDER_COUNT;
const FLAME_TRIGGER_ROOM_GUST_ROW: usize = FLAME_DEBUG_SLIDER_COUNT + 1;
const FLAME_SAVE_ROW: usize = FLAME_DEBUG_SLIDER_COUNT + FLAME_MID_ACTION_COUNT;
const FLAME_RESET_ROW: usize = FLAME_DEBUG_SLIDER_COUNT + FLAME_MID_ACTION_COUNT + 1;
const FLAME_CLOSE_ROW: usize = FLAME_DEBUG_SLIDER_COUNT + FLAME_MID_ACTION_COUNT + 2;
const FLAME_ROW_COUNT: usize = FLAME_DEBUG_SLIDER_COUNT + FLAME_MID_ACTION_COUNT + 3;
const MAX_VISIBLE_ROWS: usize = 14;

struct FlameDebugLayout {
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
    scroll_row: usize,
    visible_rows: usize,
}

impl FlameDebugLayout {
    fn compute(window_w: f32, window_h: f32, scroll_row: usize) -> Self {
        let scale = metrics::scene_scale(window_w, window_h);
        let row_h = (18.0 * scale).max(14.0);
        let row_gap = (2.0 * scale).max(1.0);
        let title_h = (22.0 * scale).max(14.0);
        let pad = (8.0 * scale).max(5.0);
        let margin = (10.0 * scale).max(6.0);
        let panel_w = (380.0 * scale).min(window_w * 0.5);
        let panel_x = margin;
        let panel_y = margin;
        let rows_y0 = panel_y + pad + title_h + pad;
        let label_w = panel_w * 0.46;
        let slider_w = panel_w * 0.28;
        let value_w = (panel_w - label_w - slider_w - 10.0 * scale).max(32.0);
        let visible_rows = FLAME_ROW_COUNT.min(MAX_VISIBLE_ROWS);
        let _ = window_h;
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
            scroll_row,
            visible_rows,
        }
    }

    fn slider_track(&self, row: usize) -> (f32, f32, f32, f32) {
        let vis = row.saturating_sub(self.scroll_row);
        let row_y = self.rows_y0 + vis as f32 * (self.row_h + self.row_gap);
        let track_x = self.panel_x + self.label_w;
        let track_h = (4.0 * self.scale).max(3.0);
        let track_y = row_y + (self.row_h - track_h) * 0.5;
        (track_x, track_y, self.slider_w, track_h)
    }

    fn value_cell(&self, row: usize) -> (f32, f32, f32, f32) {
        let vis = row.saturating_sub(self.scroll_row);
        let row_y = self.rows_y0 + vis as f32 * (self.row_h + self.row_gap);
        let x = self.panel_x + self.label_w + self.slider_w + 4.0 * self.scale;
        (x, row_y, self.value_w, self.row_h)
    }

    fn row_rect(&self, row: usize) -> (f32, f32, f32, f32) {
        let vis = row.saturating_sub(self.scroll_row);
        let row_y = self.rows_y0 + vis as f32 * (self.row_h + self.row_gap);
        (self.panel_x + 4.0, row_y, self.panel_w - 8.0, self.row_h)
    }

    fn row_visible(&self, row: usize) -> bool {
        row >= self.scroll_row && row < self.scroll_row + self.visible_rows
    }

    fn hit_row_rect(&self, row: usize) -> Option<(f32, f32, f32, f32)> {
        if row >= FLAME_ROW_COUNT || !self.row_visible(row) {
            return None;
        }
        Some(self.row_rect(row))
    }
}

fn point_in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx <= r.0 + r.2 && my >= r.1 && my <= r.1 + r.3
}

pub struct FlameDebugOverlay {
    cursor: usize,
    pub tuning: FlameTuning,
    scroll_row: usize,
    editing: bool,
    edit_buffer: String,
    dragging_slider: Option<usize>,
    pointer: DebugPointerState,
}

pub enum FlameDebugResult {
    Stay,
    Close,
    Reset,
    Save,
    TriggerGust { room: bool },
}

impl FlameDebugOverlay {
    pub fn new(tuning: FlameTuning) -> Self {
        Self {
            cursor: 0,
            tuning,
            scroll_row: 0,
            editing: false,
            edit_buffer: String::new(),
            dragging_slider: None,
            pointer: DebugPointerState::default(),
        }
    }

    fn max_scroll(&self, layout: &FlameDebugLayout) -> usize {
        FLAME_ROW_COUNT.saturating_sub(layout.visible_rows)
    }

    fn ensure_scroll(&mut self, layout: &FlameDebugLayout) {
        let max_scroll = self.max_scroll(layout);
        if self.cursor >= self.scroll_row + layout.visible_rows {
            self.scroll_row = (self.cursor + 1).saturating_sub(layout.visible_rows);
        }
        if self.cursor < self.scroll_row {
            self.scroll_row = self.cursor;
        }
        self.scroll_row = self.scroll_row.min(max_scroll);
    }

    fn apply_slider_mx(&mut self, row: usize, mx: f32, layout: &FlameDebugLayout) {
        let (tx, _, tw, _) = layout.slider_track(row);
        let (_, min, max, _) = FLAME_DEBUG_ROW_META[row];
        let t = ((mx - tx) / tw.max(1e-6)).clamp(0.0, 1.0);
        self.tuning.set_debug_row_value(row, min + t * (max - min));
    }

    fn adjust_row(&mut self, dir: f32) {
        if self.editing || self.cursor >= FLAME_DEBUG_SLIDER_COUNT {
            return;
        }
        let row = self.cursor;
        let (_, _, _, step) = FLAME_DEBUG_ROW_META[row];
        let v = self.tuning.debug_row_value(row) + dir * step;
        self.tuning.set_debug_row_value(row, v);
    }

    fn clear_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    fn begin_editing(&mut self) {
        if self.cursor >= FLAME_DEBUG_SLIDER_COUNT {
            return;
        }
        let v = self.tuning.debug_row_value(self.cursor);
        let mut s = format!("{:.6}", v);
        while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
            s.pop();
        }
        self.edit_buffer = s;
        self.editing = true;
    }

    fn commit_edit(&mut self) {
        let row = self.cursor.min(FLAME_DEBUG_SLIDER_COUNT.saturating_sub(1));
        if let Ok(v) = self.edit_buffer.trim().parse::<f32>() {
            self.tuning.set_debug_row_value(row, v);
        }
        self.clear_edit();
    }

    fn format_row_display(row: usize, v: f32) -> String {
        let (_, _, _, step) = FLAME_DEBUG_ROW_META[row];
        if step >= 0.01 {
            format!("{v:.2}")
        } else if step >= 0.001 {
            format!("{v:.3}")
        } else {
            format!("{v:.4}")
        }
    }

    fn action_label(row: usize) -> (&'static str, ButtonVariant) {
        match row {
            FLAME_TRIGGER_GUST_ROW => ("Trigger gust (per candle)", ButtonVariant::Primary),
            FLAME_TRIGGER_ROOM_GUST_ROW => ("Trigger room gust (all candles)", ButtonVariant::Primary),
            FLAME_SAVE_ROW => ("Save for shop / gameplay", ButtonVariant::Primary),
            FLAME_RESET_ROW => ("Reset to defaults", ButtonVariant::Danger),
            _ => ("Close", ButtonVariant::Subtle),
        }
    }

    fn mid_action_result(row: usize) -> Option<FlameDebugResult> {
        match row {
            FLAME_TRIGGER_GUST_ROW => Some(FlameDebugResult::TriggerGust { room: false }),
            FLAME_TRIGGER_ROOM_GUST_ROW => Some(FlameDebugResult::TriggerGust { room: true }),
            _ => None,
        }
    }

    pub fn copy_to_clipboard(&self) {
        let text = self.tuning.to_rust_literal();
        #[cfg(feature = "clipboard")]
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Err(e) = cb.set_text(&text) {
                    log::error!("Clipboard write failed: {e}");
                } else {
                    log::info!("Flame tuning snapshot copied");
                }
            }
            Err(e) => log::error!("Could not open clipboard: {e}"),
        }
        #[cfg(not(feature = "clipboard"))]
        log::info!("Flame tuning snapshot (clipboard unavailable in this build):\n{text}");
    }

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
            Scancode::Period | Scancode::KpPeriod => {
                if !self.edit_buffer.contains('.') {
                    self.edit_buffer.push('.');
                }
                true
            }
            Scancode::Minus | Scancode::KpMinus => {
                if self.edit_buffer.is_empty() {
                    self.edit_buffer.push('-');
                }
                true
            }
            _ => {
                if let Some(c) = scancode_to_digit_char(code) {
                    self.edit_buffer.push(c);
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn update(
        &mut self,
        actions: &[UiAction],
        mouse: Option<(f32, f32, bool, bool)>,
        window_w: f32,
        window_h: f32,
    ) -> FlameDebugResult {
        let layout = FlameDebugLayout::compute(window_w, window_h, self.scroll_row);
        self.ensure_scroll(&layout);
        self.pointer.sync_held(mouse);
        self.pointer.clear_hover();

        if let Some((mx, my, clicked, held)) = mouse {
            for i in 0..FLAME_ROW_COUNT {
                if let Some(rect) = layout.hit_row_rect(i)
                    && point_in_rect(mx, my, rect)
                {
                    self.pointer.hover_row = Some(i);
                    if self.dragging_slider.is_none() {
                        self.cursor = i;
                    }
                    break;
                }
            }

            if let Some(di) = self.dragging_slider
                && held
                && di < FLAME_DEBUG_SLIDER_COUNT
            {
                self.apply_slider_mx(di, mx, &layout);
            }
            if (clicked || held) && self.dragging_slider.is_none() {
                for i in self.scroll_row
                    ..(self.scroll_row + layout.visible_rows).min(FLAME_DEBUG_SLIDER_COUNT)
                {
                    if point_in_rect(mx, my, layout.slider_track(i)) {
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
                for i in self.scroll_row
                    ..(self.scroll_row + layout.visible_rows).min(FLAME_DEBUG_SLIDER_COUNT)
                {
                    if point_in_rect(mx, my, layout.value_cell(i)) {
                        self.cursor = i;
                        self.begin_editing();
                        break;
                    }
                }
                for row in FLAME_TRIGGER_GUST_ROW..=FLAME_CLOSE_ROW {
                    let Some(rect) = layout.hit_row_rect(row) else {
                        continue;
                    };
                    if point_in_rect(mx, my, rect) {
                        self.cursor = row;
                        self.clear_edit();
                        if let Some(result) = Self::mid_action_result(row) {
                            return result;
                        }
                        return match row {
                            FLAME_SAVE_ROW => FlameDebugResult::Save,
                            FLAME_RESET_ROW => FlameDebugResult::Reset,
                            _ => FlameDebugResult::Close,
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
                    self.cursor = (self.cursor + 1) % FLAME_ROW_COUNT;
                    self.clear_edit();
                    self.ensure_scroll(&layout);
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + FLAME_ROW_COUNT - 1) % FLAME_ROW_COUNT;
                    self.clear_edit();
                    self.ensure_scroll(&layout);
                }
                UiAction::FocusNext | UiAction::FocusPrev => {
                    if !self.editing {
                        self.adjust_row(if matches!(a, UiAction::FocusPrev) {
                            -1.0
                        } else {
                            1.0
                        });
                    }
                }
                UiAction::Confirm | UiAction::CommitDiscard => {
                    if self.editing {
                        self.commit_edit();
                    } else if let Some(result) = Self::mid_action_result(self.cursor) {
                        return result;
                    } else if self.cursor == FLAME_SAVE_ROW {
                        return FlameDebugResult::Save;
                    } else if self.cursor == FLAME_RESET_ROW {
                        return FlameDebugResult::Reset;
                    } else if self.cursor == FLAME_CLOSE_ROW {
                        return FlameDebugResult::Close;
                    } else if self.cursor < FLAME_DEBUG_SLIDER_COUNT {
                        self.begin_editing();
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    if self.editing {
                        self.clear_edit();
                    } else {
                        return FlameDebugResult::Close;
                    }
                }
                _ => {}
            }
        }
        FlameDebugResult::Stay
    }

    pub fn draw(&self, window_w: f32, window_h: f32) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let layout = FlameDebugLayout::compute(window_w, window_h, self.scroll_row);
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        let pad = (8.0 * layout.scale).max(5.0);
        let title_h = (22.0 * layout.scale).max(14.0);
        let hint_h = (13.0 * layout.scale).max(9.0);
        let vis_h = layout.visible_rows as f32 * (layout.row_h + layout.row_gap);
        let panel_h = pad + title_h + pad + vis_h + pad + hint_h * 2.0 + pad * 2.0;
        let row_font = typography::tier_at_most(layout.row_h * 0.48, window_h);

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
            text: "Candle flames — plume + placement".into(),
            color: color::JADE,
            ..Default::default()
        });

        for (i, (name, min, max, _)) in FLAME_DEBUG_ROW_META
            .iter()
            .enumerate()
            .skip(self.scroll_row)
            .take(
                layout
                    .visible_rows
                    .min(FLAME_DEBUG_SLIDER_COUNT.saturating_sub(self.scroll_row)),
            )
        {
            if !layout.row_visible(i) {
                continue;
            }
            let visual = DebugRowVisual::for_row(i, self.cursor, &self.pointer);
            let v = self.tuning.debug_row_value(i);
            let (rx, ry, rw, rh) = layout.row_rect(i);
            let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, ButtonVariant::Default);
            instances.push(GpuInstance {
                rect: [rx, ry, rw, rh],
                color: bg,
                user: 0,
            });
            labels.push(TextLabel {
                rect: [
                    layout.panel_x + 6.0 * layout.scale,
                    ry,
                    layout.label_w,
                    layout.row_h,
                ],
                text: name.to_string(),
                font_px: Some(row_font),
                color: tc,
                align: TextAlign::Left,
                ..Default::default()
            });

            let (track_x, track_y, tw, th) = layout.slider_track(i);
            let t = ((v - min) / (max - min).max(1e-8)).clamp(0.0, 1.0);
            let fill_w = tw * t;
            instances.push(GpuInstance {
                rect: [track_x, track_y, tw, th],
                color: color::WALNUT_INK,
                user: 0,
            });
            instances.push(GpuInstance {
                rect: [track_x, track_y, fill_w, th],
                color: if visual.highlighted {
                    color::JADE
                } else {
                    color::alpha(color::JADE, 0.7)
                },
                user: 0,
            });
            let knob_size = th * 2.5;
            instances.push(GpuInstance {
                rect: [
                    track_x + fill_w - knob_size * 0.5,
                    track_y + (th - knob_size) * 0.5,
                    knob_size,
                    knob_size,
                ],
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
            labels.push(TextLabel {
                rect: [vx + 2.0 * layout.scale, vy, vw - 4.0 * layout.scale, vh],
                text: value_text,
                font_px: Some(row_font),
                color: [tc[0] * 0.92, tc[1] * 0.92, tc[2] * 0.92, 1.0],
                align: TextAlign::Right,
                ..Default::default()
            });
        }

        for row in FLAME_TRIGGER_GUST_ROW..=FLAME_CLOSE_ROW {
            if !layout.row_visible(row) {
                continue;
            }
            let (label, variant) = Self::action_label(row);
            let (rx, ry, rw, rh) = layout.row_rect(row);
            let visual = DebugRowVisual::for_row(row, self.cursor, &self.pointer);
            let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, variant);
            instances.push(GpuInstance {
                rect: [rx, ry, rw, rh],
                color: bg,
                user: 0,
            });
            labels.push(TextLabel {
                rect: [layout.panel_x, ry, layout.panel_w, rh],
                text: label.into(),
                font_px: Some(row_font),
                color: tc,
                align: TextAlign::Center,
                ..Default::default()
            });
        }

        let hint_y = layout.panel_y + panel_h - pad - hint_h * 2.0;
        let hint_font = typography::tier_at_most(hint_h * 0.85, window_h);
        labels.push(TextLabel {
            rect: [layout.panel_x, hint_y, layout.panel_w, hint_h],
            text: "↑↓ navigate · ←→ adjust · Enter edit/trigger".into(),
            font_px: Some(hint_font),
            color: color::alpha(color::PARCHMENT, 0.55),
            align: TextAlign::Center,
            ..Default::default()
        });
        labels.push(TextLabel {
            rect: [layout.panel_x, hint_y + hint_h, layout.panel_w, hint_h],
            text: "Ctrl+C copy Rust literal".into(),
            font_px: Some(hint_font),
            color: color::alpha(color::PARCHMENT, 0.55),
            align: TextAlign::Center,
            ..Default::default()
        });

        (instances, labels)
    }
}

fn scancode_to_digit_char(code: Scancode) -> Option<char> {
    Some(match code {
        Scancode::_0 | Scancode::Kp0 => '0',
        Scancode::_1 | Scancode::Kp1 => '1',
        Scancode::_2 | Scancode::Kp2 => '2',
        Scancode::_3 | Scancode::Kp3 => '3',
        Scancode::_4 | Scancode::Kp4 => '4',
        Scancode::_5 | Scancode::Kp5 => '5',
        Scancode::_6 | Scancode::Kp6 => '6',
        Scancode::_7 | Scancode::Kp7 => '7',
        Scancode::_8 | Scancode::Kp8 => '8',
        Scancode::_9 | Scancode::Kp9 => '9',
        _ => return None,
    })
}
