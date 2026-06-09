//! Debug slider panel for victory run-summary 3D moon rotation + phase.

use sdl3::keyboard::Scancode;

use crate::debug_overlay_ui::{self, DebugPointerState, DebugRowVisual};
use crate::theme::{ButtonVariant, color, metrics, typography};
use crate::victory_moon_tuning::{
    VICTORY_MOON_ACTION_CLOSE, VICTORY_MOON_ACTION_COPY, VICTORY_MOON_ACTION_FIRST_QUARTER,
    VICTORY_MOON_ACTION_FULL, VICTORY_MOON_ACTION_LAST_QUARTER, VICTORY_MOON_ACTION_LIVE,
    VICTORY_MOON_ACTION_NEW, VICTORY_MOON_ACTION_RESET, VICTORY_MOON_DEBUG_ROW_COUNT,
    VICTORY_MOON_DEBUG_ROW_META, VICTORY_MOON_DEBUG_SLIDER_COUNT, VictoryMoonDebug,
};
use crate::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use mahjuro_types::UiAction;

const VISIBLE_ROWS: usize = 14;

struct VictoryMoonDebugLayout {
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
}

impl VictoryMoonDebugLayout {
    fn compute(window_w: f32, window_h: f32, scroll_row: usize) -> Self {
        let scale = metrics::scene_scale(window_w, window_h);
        let row_h = (18.0 * scale).max(14.0);
        let row_gap = (2.0 * scale).max(1.0);
        let title_h = (22.0 * scale).max(14.0);
        let pad = (8.0 * scale).max(5.0);
        let margin = (10.0 * scale).max(6.0);
        let panel_w = (380.0 * scale).min(window_w * 0.48);
        let panel_x = margin;
        let panel_y = margin;
        let rows_y0 = panel_y + pad + title_h + pad;
        let label_w = panel_w * 0.44;
        let slider_w = panel_w * 0.30;
        let value_w = (panel_w - label_w - slider_w - 10.0 * scale).max(32.0);
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

    fn action_row_rect(&self, row: usize) -> Option<(f32, f32, f32, f32)> {
        if row < VICTORY_MOON_DEBUG_SLIDER_COUNT {
            return None;
        }
        Some(self.row_rect(row))
    }

    fn hit_row_rect(&self, row: usize) -> Option<(f32, f32, f32, f32)> {
        if row >= VICTORY_MOON_DEBUG_ROW_COUNT {
            return None;
        }
        if row < VICTORY_MOON_DEBUG_SLIDER_COUNT {
            return Some(self.row_rect(row));
        }
        self.action_row_rect(row)
    }
}

fn point_in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx <= r.0 + r.2 && my >= r.1 && my <= r.1 + r.3
}

pub struct VictoryMoonDebugOverlay {
    cursor: usize,
    pub debug: VictoryMoonDebug,
    scroll_row: usize,
    editing: bool,
    edit_buffer: String,
    dragging_slider: Option<usize>,
    pointer: DebugPointerState,
}

pub enum VictoryMoonDebugResult {
    Stay,
    Close,
    Reset,
}

impl VictoryMoonDebugOverlay {
    pub fn new(debug: VictoryMoonDebug) -> Self {
        Self {
            cursor: 0,
            debug,
            scroll_row: 0,
            editing: false,
            edit_buffer: String::new(),
            dragging_slider: None,
            pointer: DebugPointerState::default(),
        }
    }

    fn ensure_scroll(&mut self) {
        if self.cursor >= self.scroll_row + VISIBLE_ROWS {
            self.scroll_row = self.cursor + 1 - VISIBLE_ROWS;
        }
        if self.cursor < self.scroll_row {
            self.scroll_row = self.cursor;
        }
    }

    fn apply_slider_mx(&mut self, row: usize, mx: f32, layout: &VictoryMoonDebugLayout) {
        let (tx, _, tw, _) = layout.slider_track(row);
        let (_, min, max, _) = VICTORY_MOON_DEBUG_ROW_META[row];
        let t = ((mx - tx) / tw.max(1e-6)).clamp(0.0, 1.0);
        self.debug.set_debug_row_value(row, min + t * (max - min));
    }

    fn adjust_row(&mut self, dir: f32) {
        if self.editing || self.cursor >= VICTORY_MOON_DEBUG_SLIDER_COUNT {
            return;
        }
        if self.cursor == 3 && self.debug.moon_phase.use_live_calendar {
            return;
        }
        let row = self.cursor;
        let (_, _, _, step) = VICTORY_MOON_DEBUG_ROW_META[row];
        let v = self.debug.debug_row_value(row) + dir * step;
        self.debug.set_debug_row_value(row, v);
    }

    fn clear_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    fn begin_editing(&mut self) {
        if self.cursor >= VICTORY_MOON_DEBUG_SLIDER_COUNT {
            return;
        }
        if self.cursor == 3 && self.debug.moon_phase.use_live_calendar {
            return;
        }
        let v = self.debug.debug_row_value(self.cursor);
        let mut s = format!("{:.6}", v);
        while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
            s.pop();
        }
        self.edit_buffer = s;
        self.editing = true;
    }

    fn commit_edit(&mut self) {
        let row = self
            .cursor
            .min(VICTORY_MOON_DEBUG_SLIDER_COUNT.saturating_sub(1));
        if let Ok(v) = self.edit_buffer.trim().parse::<f32>() {
            self.debug.set_debug_row_value(row, v);
        }
        self.clear_edit();
    }

    fn format_row_display(row: usize, v: f32) -> String {
        let (_, _, _, step) = VICTORY_MOON_DEBUG_ROW_META[row];
        if step >= 0.01 {
            format!("{v:.2}")
        } else if step >= 0.001 {
            format!("{v:.3}")
        } else {
            format!("{v:.4}")
        }
    }

    fn action_label(&self, row: usize) -> (&'static str, ButtonVariant) {
        match row {
            VICTORY_MOON_ACTION_LIVE => (
                if self.debug.moon_phase.use_live_calendar {
                    "Use live calendar phase [ON]"
                } else {
                    "Use live calendar phase [OFF]"
                },
                ButtonVariant::Default,
            ),
            VICTORY_MOON_ACTION_NEW => ("New moon (0.00)", ButtonVariant::Default),
            VICTORY_MOON_ACTION_FIRST_QUARTER => ("First quarter (0.25)", ButtonVariant::Default),
            VICTORY_MOON_ACTION_FULL => ("Full moon (0.50)", ButtonVariant::Default),
            VICTORY_MOON_ACTION_LAST_QUARTER => ("Last quarter (0.75)", ButtonVariant::Default),
            VICTORY_MOON_ACTION_COPY => ("Copy rotation Rust literal", ButtonVariant::Primary),
            VICTORY_MOON_ACTION_RESET => ("Reset to defaults", ButtonVariant::Danger),
            VICTORY_MOON_ACTION_CLOSE => ("Close", ButtonVariant::Subtle),
            _ => ("", ButtonVariant::Default),
        }
    }

    fn dispatch_action(&mut self, row: usize) -> VictoryMoonDebugResult {
        match row {
            VICTORY_MOON_ACTION_LIVE => {
                self.debug.toggle_live_calendar();
                VictoryMoonDebugResult::Stay
            }
            VICTORY_MOON_ACTION_NEW => {
                self.debug.set_moon_phase_preset(0.0);
                VictoryMoonDebugResult::Stay
            }
            VICTORY_MOON_ACTION_FIRST_QUARTER => {
                self.debug.set_moon_phase_preset(0.25);
                VictoryMoonDebugResult::Stay
            }
            VICTORY_MOON_ACTION_FULL => {
                self.debug.set_moon_phase_preset(0.5);
                VictoryMoonDebugResult::Stay
            }
            VICTORY_MOON_ACTION_LAST_QUARTER => {
                self.debug.set_moon_phase_preset(0.75);
                VictoryMoonDebugResult::Stay
            }
            VICTORY_MOON_ACTION_COPY => {
                self.copy_to_clipboard();
                VictoryMoonDebugResult::Stay
            }
            VICTORY_MOON_ACTION_RESET => VictoryMoonDebugResult::Reset,
            VICTORY_MOON_ACTION_CLOSE => VictoryMoonDebugResult::Close,
            _ => VictoryMoonDebugResult::Stay,
        }
    }

    pub fn copy_to_clipboard(&self) {
        let text = self.debug.to_rust_literal();
        #[cfg(feature = "clipboard")]
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Err(e) = cb.set_text(&text) {
                    log::error!("Clipboard write failed: {e}");
                } else {
                    log::info!("Victory moon debug snapshot copied");
                }
            }
            Err(e) => log::error!("Could not open clipboard: {e}"),
        }
        #[cfg(not(feature = "clipboard"))]
        log::info!("Victory moon debug snapshot (clipboard unavailable):\n{text}");
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
    ) -> VictoryMoonDebugResult {
        self.ensure_scroll();
        let layout = VictoryMoonDebugLayout::compute(window_w, window_h, self.scroll_row);
        self.pointer.sync_held(mouse);
        self.pointer.clear_hover();

        if let Some((mx, my, clicked, held)) = mouse {
            for i in 0..VICTORY_MOON_DEBUG_ROW_COUNT {
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
                && di < VICTORY_MOON_DEBUG_SLIDER_COUNT
            {
                self.apply_slider_mx(di, mx, &layout);
            }
            if (clicked || held) && self.dragging_slider.is_none() {
                for i in self.scroll_row
                    ..(self.scroll_row + VISIBLE_ROWS).min(VICTORY_MOON_DEBUG_SLIDER_COUNT)
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
                    ..(self.scroll_row + VISIBLE_ROWS).min(VICTORY_MOON_DEBUG_SLIDER_COUNT)
                {
                    if point_in_rect(mx, my, layout.value_cell(i)) {
                        self.cursor = i;
                        self.begin_editing();
                        break;
                    }
                }
                for row in VICTORY_MOON_DEBUG_SLIDER_COUNT..VICTORY_MOON_DEBUG_ROW_COUNT {
                    let Some(rect) = layout.action_row_rect(row) else {
                        continue;
                    };
                    if point_in_rect(mx, my, rect) {
                        self.cursor = row;
                        self.clear_edit();
                        return self.dispatch_action(row);
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
                    self.cursor = (self.cursor + 1) % VICTORY_MOON_DEBUG_ROW_COUNT;
                    self.clear_edit();
                    self.ensure_scroll();
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + VICTORY_MOON_DEBUG_ROW_COUNT - 1)
                        % VICTORY_MOON_DEBUG_ROW_COUNT;
                    self.clear_edit();
                    self.ensure_scroll();
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
                    } else if self.cursor >= VICTORY_MOON_DEBUG_SLIDER_COUNT {
                        return self.dispatch_action(self.cursor);
                    } else if self.cursor < VICTORY_MOON_DEBUG_SLIDER_COUNT {
                        self.begin_editing();
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    if self.editing {
                        self.clear_edit();
                    } else {
                        return VictoryMoonDebugResult::Close;
                    }
                }
                _ => {}
            }
        }
        VictoryMoonDebugResult::Stay
    }

    pub fn draw(&self, window_w: f32, window_h: f32) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let layout = VictoryMoonDebugLayout::compute(window_w, window_h, self.scroll_row);
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        let pad = (8.0 * layout.scale).max(5.0);
        let title_h = (22.0 * layout.scale).max(14.0);
        let hint_h = (13.0 * layout.scale).max(9.0);
        let vis_h = VISIBLE_ROWS as f32 * (layout.row_h + layout.row_gap);
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
            text: "Victory 3D moon".into(),
            color: color::JADE,
            ..Default::default()
        });

        for (i, (name, min, max, _)) in VICTORY_MOON_DEBUG_ROW_META
            .iter()
            .enumerate()
            .skip(self.scroll_row)
            .take(VISIBLE_ROWS.min(VICTORY_MOON_DEBUG_SLIDER_COUNT.saturating_sub(self.scroll_row)))
        {
            let visual = DebugRowVisual::for_row(i, self.cursor, &self.pointer);
            let v = self.debug.debug_row_value(i);
            let live_phase = i == 3 && self.debug.moon_phase.use_live_calendar;
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
                text: if live_phase {
                    format!("{name} (live)")
                } else {
                    (*name).to_string()
                },
                font_px: Some(row_font),
                color: if live_phase {
                    color::alpha(tc, 0.55)
                } else {
                    tc
                },
                align: TextAlign::Left,
                ..Default::default()
            });

            if !live_phase {
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
            }

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

        for row in VICTORY_MOON_DEBUG_SLIDER_COUNT..VICTORY_MOON_DEBUG_ROW_COUNT {
            let vis = row.saturating_sub(self.scroll_row);
            if vis >= VISIBLE_ROWS {
                continue;
            }
            let (label, variant) = self.action_label(row);
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
            text: "↑↓ navigate · ←→ adjust · Enter edit/confirm".into(),
            font_px: Some(hint_font),
            color: color::alpha(color::PARCHMENT, 0.55),
            align: TextAlign::Center,
            ..Default::default()
        });
        labels.push(TextLabel {
            rect: [layout.panel_x, hint_y + hint_h, layout.panel_w, hint_h],
            text: "Ctrl+C copy rotation literal".into(),
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
