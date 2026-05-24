//! Debug slider panel for [`super::rain_tuning::RainTuning`] — main-menu CPU rain field.

use sdl3::keyboard::Scancode;

use crate::render::rain_tuning::{
    RAIN_DEBUG_ROW_META, RAIN_DEBUG_SLIDER_COUNT, RainTuning, rain_color_swatch_rgb,
    rain_hue_wheel_preview_linear, rain_row_is_hue, rain_row_is_saturation,
};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::input::UiAction;

const RAIN_SAVE_ROW: usize = RAIN_DEBUG_SLIDER_COUNT;
const RAIN_RESET_ROW: usize = RAIN_DEBUG_SLIDER_COUNT + 1;
const RAIN_CLOSE_ROW: usize = RAIN_DEBUG_SLIDER_COUNT + 2;
const RAIN_ROW_COUNT: usize = RAIN_DEBUG_SLIDER_COUNT + 3;
const VISIBLE_ROWS: usize = 22;

#[derive(Clone, Copy)]
struct RainDebugLayout {
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

impl RainDebugLayout {
    fn compute(window_w: f32, window_h: f32, scroll_row: usize) -> Self {
        let scale = metrics::scene_scale(window_w, window_h);
        let row_h = (18.0 * scale).max(14.0);
        let row_gap = (2.0 * scale).max(1.0);
        let title_h = (22.0 * scale).max(14.0);
        let pad = (8.0 * scale).max(5.0);
        let margin = (10.0 * scale).max(6.0);
        let panel_w = (340.0 * scale).min(window_w * 0.46);
        let panel_x = window_w - panel_w - margin;
        let panel_y = margin;
        let rows_y0 = panel_y + pad + title_h + pad;
        let label_w = panel_w * 0.42;
        let slider_w = panel_w * 0.32;
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
}

fn point_in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx <= r.0 + r.2 && my >= r.1 && my <= r.1 + r.3
}

pub struct RainDebugOverlay {
    cursor: usize,
    pub tuning: RainTuning,
    scroll_row: usize,
    editing: bool,
    edit_buffer: String,
    dragging_slider: Option<usize>,
}

pub enum RainDebugResult {
    Stay,
    Close,
    Reset,
    Save,
}

impl RainDebugOverlay {
    pub fn new(tuning: RainTuning) -> Self {
        Self {
            cursor: 0,
            tuning,
            scroll_row: 0,
            editing: false,
            edit_buffer: String::new(),
            dragging_slider: None,
        }
    }

    fn row_count(&self) -> usize {
        RAIN_ROW_COUNT
    }

    fn ensure_scroll(&mut self) {
        if self.cursor >= self.scroll_row + VISIBLE_ROWS {
            self.scroll_row = self.cursor + 1 - VISIBLE_ROWS;
        }
        if self.cursor < self.scroll_row {
            self.scroll_row = self.cursor;
        }
    }

    fn row_value(&self, row: usize) -> f32 {
        self.tuning.debug_row_value(row)
    }

    fn set_row_value(&mut self, row: usize, v: f32) {
        self.tuning.set_debug_row_value(row, v);
    }

    fn apply_slider_mx(&mut self, row: usize, mx: f32, layout: &RainDebugLayout) {
        let (tx, _, tw, _) = layout.slider_track(row);
        let (_, min, max, _) = RAIN_DEBUG_ROW_META[row];
        let t = ((mx - tx) / tw.max(1e-6)).clamp(0.0, 1.0);
        self.set_row_value(row, min + t * (max - min));
    }

    fn adjust_row(&mut self, dir: f32) {
        if self.editing || self.cursor >= RAIN_DEBUG_SLIDER_COUNT {
            return;
        }
        let row = self.cursor;
        let (_, _, _, step) = RAIN_DEBUG_ROW_META[row];
        self.set_row_value(row, self.row_value(row) + dir * step);
    }

    fn clear_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    fn begin_editing(&mut self) {
        let row = self.cursor;
        let v = self.row_value(row);
        self.edit_buffer = if rain_row_is_hue(row) {
            format!("{}", (v * 360.0).round() as i32)
        } else if rain_row_is_saturation(row) {
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
        let row = self.cursor.min(RAIN_DEBUG_SLIDER_COUNT.saturating_sub(1));
        let t = self.edit_buffer.trim();
        let parsed = if rain_row_is_hue(row) {
            t.parse::<f32>()
                .ok()
                .map(|deg| (deg / 360.0).fract())
                .or_else(|| t.parse::<f32>().ok().map(|h| h.fract()))
        } else if rain_row_is_saturation(row) {
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

    fn format_row_display(row: usize, v: f32) -> String {
        let (_, _, _, step) = RAIN_DEBUG_ROW_META[row];
        if rain_row_is_hue(row) {
            return format!("{}°", (v * 360.0).round() as i32);
        }
        if rain_row_is_saturation(row) {
            return format!("{:.0}%", v * 100.0);
        }
        if step >= 0.01 {
            format!("{v:.2}")
        } else if step >= 0.001 {
            format!("{v:.3}")
        } else {
            format!("{v:.4}")
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
            let preview = rain_hue_wheel_preview_linear(h);
            instances.push(GpuInstance {
                rect: [track_x + seg_w * i as f32, track_y, seg_w + 0.5, th],
                color: [preview[0], preview[1], preview[2], 1.0],
                user: 0,
            });
        }
    }

    fn draw_action_row(
        &self,
        layout: &RainDebugLayout,
        row_y: f32,
        row: usize,
        label: &str,
        instances: &mut Vec<GpuInstance>,
        labels: &mut Vec<TextLabel>,
        row_font: f32,
    ) {
        let is_focused = self.cursor == row;
        instances.push(GpuInstance {
            rect: [
                layout.panel_x + 4.0,
                row_y,
                layout.panel_w - 8.0,
                layout.row_h,
            ],
            color: if is_focused {
                color::alpha(color::WALNUT_SOFT, 0.95)
            } else {
                color::alpha(color::WALNUT_DEEP, 0.85)
            },
            user: 0,
        });
        labels.push(TextLabel {
            rect: [layout.panel_x, row_y, layout.panel_w, layout.row_h],
            text: label.into(),
            font_px: Some(row_font),
            color: if is_focused {
                color::PARCHMENT
            } else {
                color::alpha(color::JADE, 0.7)
            },
            align: TextAlign::Center,
            ..Default::default()
        });
    }

    pub fn copy_to_clipboard(&self) {
        let text = self.tuning.to_rust_literal();
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Err(e) = cb.set_text(&text) {
                    log::error!("Clipboard write failed: {e}");
                } else {
                    log::info!("Rain tuning snapshot copied");
                }
            }
            Err(e) => log::error!("Could not open clipboard: {e}"),
        }
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
    ) -> RainDebugResult {
        self.ensure_scroll();
        let layout = RainDebugLayout::compute(window_w, window_h, self.scroll_row);

        if let Some((mx, my, clicked, held)) = mouse {
            if let Some(di) = self.dragging_slider
                && held
                && di < RAIN_DEBUG_SLIDER_COUNT
            {
                self.apply_slider_mx(di, mx, &layout);
            }
            if (clicked || held) && self.dragging_slider.is_none() {
                for i in
                    self.scroll_row..(self.scroll_row + VISIBLE_ROWS).min(RAIN_DEBUG_SLIDER_COUNT)
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
                for i in
                    self.scroll_row..(self.scroll_row + VISIBLE_ROWS).min(RAIN_DEBUG_SLIDER_COUNT)
                {
                    if point_in_rect(mx, my, layout.value_cell(i)) {
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
                    self.ensure_scroll();
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + self.row_count() - 1) % self.row_count();
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
                    } else if self.cursor == RAIN_SAVE_ROW {
                        return RainDebugResult::Save;
                    } else if self.cursor == RAIN_RESET_ROW {
                        return RainDebugResult::Reset;
                    } else if self.cursor == RAIN_CLOSE_ROW {
                        return RainDebugResult::Close;
                    } else if self.cursor < RAIN_DEBUG_SLIDER_COUNT {
                        self.begin_editing();
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    if self.editing {
                        self.clear_edit();
                    } else {
                        return RainDebugResult::Close;
                    }
                }
                _ => {}
            }
        }
        RainDebugResult::Stay
    }

    pub fn draw(&self, window_w: f32, window_h: f32) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let layout = RainDebugLayout::compute(window_w, window_h, self.scroll_row);
        let mut instances = Vec::new();
        let mut labels = Vec::new();
        let pad = (8.0 * layout.scale).max(5.0);
        let title_h = (22.0 * layout.scale).max(14.0);
        let hint_h = (13.0 * layout.scale).max(9.0);
        let vis_h = VISIBLE_ROWS as f32 * (layout.row_h + layout.row_gap) + layout.row_h * 3.0;
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
            text: "Rain field (main menu)".into(),
            color: color::JADE,
            ..Default::default()
        });

        for (i, (name, min, max, _)) in RAIN_DEBUG_ROW_META
            .iter()
            .enumerate()
            .skip(self.scroll_row)
            .take(VISIBLE_ROWS.min(RAIN_DEBUG_SLIDER_COUNT.saturating_sub(self.scroll_row)))
        {
            let is_focused = self.cursor == i;
            let v = self.row_value(i);

            let bg = if is_focused {
                color::alpha(color::WALNUT_SOFT, 0.95)
            } else {
                color::alpha(color::WALNUT_DEEP, 0.80)
            };
            let (rx, ry, rw, rh) = layout.row_rect(i);
            instances.push(GpuInstance {
                rect: [rx, ry, rw, rh],
                color: bg,
                user: 0,
            });

            let tc = if is_focused {
                color::PARCHMENT
            } else {
                color::alpha(color::STONE, 0.95)
            };

            let swatch = (14.0 * layout.scale).max(10.0);
            let mut label_x = layout.panel_x + 6.0 * layout.scale;
            let mut label_w = layout.label_w - 4.0 * layout.scale;
            if let Some(rgb) = rain_color_swatch_rgb(&self.tuning, i) {
                let sw_y = ry + (layout.row_h - swatch) * 0.5;
                instances.push(GpuInstance {
                    rect: [label_x - 1.0, sw_y - 1.0, swatch + 2.0, swatch + 2.0],
                    color: color::alpha(color::PARCHMENT, if is_focused { 0.55 } else { 0.28 }),
                    user: 0,
                });
                instances.push(GpuInstance {
                    rect: [label_x, sw_y, swatch, swatch],
                    color: [rgb[0], rgb[1], rgb[2], 1.0],
                    user: 0,
                });
                label_x += swatch + 4.0 * layout.scale;
                label_w -= swatch + 4.0 * layout.scale;
            }
            labels.push(TextLabel {
                rect: [label_x, ry, label_w, layout.row_h],
                text: name.to_string(),
                font_px: Some(row_font),
                color: tc,
                align: TextAlign::Left,
                ..Default::default()
            });

            let (track_x, track_y, tw, th) = layout.slider_track(i);
            let t = ((v - min) / (max - min).max(1e-8)).clamp(0.0, 1.0);
            let fill_w = tw * t;
            if rain_row_is_hue(i) {
                Self::draw_hue_slider_track(track_x, track_y, tw, th, &mut instances);
            } else {
                instances.push(GpuInstance {
                    rect: [track_x, track_y, tw, th],
                    color: color::WALNUT_INK,
                    user: 0,
                });
                let fill_color = rain_color_swatch_rgb(&self.tuning, i)
                    .map(|[r, g, b]| {
                        if is_focused {
                            [r, g, b, 0.95]
                        } else {
                            [r * 0.85, g * 0.85, b * 0.85, 0.75]
                        }
                    })
                    .unwrap_or_else(|| {
                        if is_focused {
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
                color: if is_focused {
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
                    if is_focused { "\u{258c}" } else { "" }
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
                font_px: Some(row_font),
                color: [tc[0] * 0.92, tc[1] * 0.92, tc[2] * 0.92, 1.0],
                align: TextAlign::Right,
                ..Default::default()
            });
        }

        let actions_y0 =
            layout.rows_y0 + VISIBLE_ROWS as f32 * (layout.row_h + layout.row_gap) + pad;
        for (idx, (row, label)) in [
            (RAIN_SAVE_ROW, "Save for main menu"),
            (RAIN_RESET_ROW, "Reset to defaults"),
            (RAIN_CLOSE_ROW, "Close"),
        ]
        .into_iter()
        .enumerate()
        {
            let row_y = actions_y0 + idx as f32 * (layout.row_h + layout.row_gap);
            self.draw_action_row(
                &layout,
                row_y,
                row,
                label,
                &mut instances,
                &mut labels,
                row_font,
            );
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
