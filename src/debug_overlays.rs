//! Debug overlay panels: visibility toggles, cascade tuning, SFX test,
//! camera parameter editor.  All are App-owned modal overlays that
//! intercept input while open.

use crate::audio;
use crate::game::cascade::CascadeTuning;
use crate::render::draw_cmd::CameraParams;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

// ── Debug visibility overlay ────────────────────────────────────────────

pub const DEBUG_VIS_ROW_COUNT: usize = 5;

pub struct DebugVisibilityOverlay {
    cursor: usize,
    pub hide_tiles: bool,
    pub hide_candles: bool,
    pub hide_blind_plaque: bool,
    pub hide_scoring_placard: bool,
    pub hide_inventory: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DebugVisResult {
    Stay,
    Close,
}

impl DebugVisibilityOverlay {
    pub fn new(
        hide_tiles: bool,
        hide_candles: bool,
        hide_blind_plaque: bool,
        hide_scoring_placard: bool,
        hide_inventory: bool,
    ) -> Self {
        Self {
            cursor: 0,
            hide_tiles,
            hide_candles,
            hide_blind_plaque,
            hide_scoring_placard,
            hide_inventory,
        }
    }

    pub fn update(&mut self, actions: &[UiAction]) -> DebugVisResult {
        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % DEBUG_VIS_ROW_COUNT;
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + DEBUG_VIS_ROW_COUNT - 1) % DEBUG_VIS_ROW_COUNT;
                }
                UiAction::Confirm => {
                    self.toggle_current();
                }
                UiAction::Cancel | UiAction::Pause => {
                    return DebugVisResult::Close;
                }
                _ => {}
            }
        }
        DebugVisResult::Stay
    }

    fn toggle_current(&mut self) {
        let f = match self.cursor {
            0 => &mut self.hide_tiles,
            1 => &mut self.hide_candles,
            2 => &mut self.hide_blind_plaque,
            3 => &mut self.hide_scoring_placard,
            4 => &mut self.hide_inventory,
            _ => return,
        };
        *f = !*f;
    }

    fn row(&self, i: usize) -> (&'static str, bool) {
        match i {
            0 => ("Hand Tiles", self.hide_tiles),
            1 => ("Candles", self.hide_candles),
            2 => ("Blind Plaque", self.hide_blind_plaque),
            3 => ("Scoring Placard", self.hide_scoring_placard),
            4 => ("Inventory + Items", self.hide_inventory),
            _ => ("", false),
        }
    }

    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        ui_scale: f32,
    ) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let scale = (window_w.min(window_h)) / 600.0 * ui_scale;
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        // Dim full-screen backdrop.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: [0.0, 0.0, 0.0, 0.7],
        });

        let panel_w = (440.0 * scale).min(window_w * 0.90);
        let row_h = (44.0 * scale).max(28.0);
        let row_gap = (8.0 * scale).max(4.0);
        let title_h = (48.0 * scale).max(28.0);
        let footer_h = (22.0 * scale).max(14.0);
        let panel_h = title_h
            + row_gap
            + DEBUG_VIS_ROW_COUNT as f32 * (row_h + row_gap)
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
            color: [0.3, 0.45, 0.7, 0.85],
        });
        // Panel.
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: [0.08, 0.08, 0.14, 0.95],
        });

        // Title.
        labels.push(TextLabel {
            rect: [panel_x, panel_y + row_gap, panel_w, title_h],
            text: "Debug Visibility".into(),
            color: [1.0, 0.95, 0.7, 1.0],
            ..Default::default()
        });

        let mut row_y = panel_y + row_gap + title_h + row_gap;
        let row_pad = 12.0 * scale;
        let check_size = row_h * 0.55;
        for i in 0..DEBUG_VIS_ROW_COUNT {
            let (name, checked) = self.row(i);
            let is_focused = self.cursor == i;

            // Row background.
            let bg = if is_focused {
                [0.20, 0.32, 0.50, 0.90]
            } else {
                [0.12, 0.15, 0.24, 0.75]
            };
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h],
                color: bg,
            });

            // Checkbox border.
            let cb_x = panel_x + row_pad;
            let cb_y = row_y + (row_h - check_size) * 0.5;
            instances.push(GpuInstance {
                rect: [cb_x - 2.0, cb_y - 2.0, check_size + 4.0, check_size + 4.0],
                color: [0.55, 0.65, 0.85, 0.9],
            });
            instances.push(GpuInstance {
                rect: [cb_x, cb_y, check_size, check_size],
                color: [0.04, 0.05, 0.10, 1.0],
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
                    color: [0.95, 0.80, 0.30, 1.0],
                });
            }

            // Label.
            let tc = if is_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.7, 0.72, 0.82, 0.9]
            };
            labels.push(TextLabel {
                rect: [
                    cb_x + check_size + row_pad,
                    row_y,
                    panel_w - (check_size + row_pad * 3.0),
                    row_h,
                ],
                text: name.to_string(),
                color: tc,
                ..Default::default()
            });

            row_y += row_h + row_gap;
        }

        // Footer hint.
        labels.push(TextLabel {
            rect: [panel_x, row_y + row_gap, panel_w, footer_h],
            text: "\u{2191}/\u{2193} select   \u{23ce} toggle   Esc close".into(),
            color: [0.55, 0.6, 0.75, 0.9],
            ..Default::default()
        });

        (instances, labels)
    }
}

// ── Cascade tuning overlay ──────────────────────────────────────────────

pub const TUNING_ROW_COUNT: usize = 10; // 9 sliders + Export button
const TUNING_SLIDER_ROWS: usize = TUNING_ROW_COUNT - 1;
const TUNING_MIN_MS: u64 = 50;
const TUNING_MAX_MS: u64 = 5000;
const TUNING_STEP_MS: u64 = 50;

pub struct TuningOverlay {
    cursor: usize,
    pub tuning: CascadeTuning,
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
        }
    }

    pub fn update(&mut self, actions: &[UiAction]) -> TuningResult {
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
            4 => &mut self.tuning.depart_lifetime_ms,
            5 => &mut self.tuning.draw_settle_ms,
            6 => &mut self.tuning.sort_settle_ms,
            7 => &mut self.tuning.wind_delay_ms,
            8 => &mut self.tuning.wind_duration_ms,
            _ => return,
        };
        *field = (*field as i64 + delta).clamp(TUNING_MIN_MS as i64, TUNING_MAX_MS as i64) as u64;
    }

    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        ui_scale: f32,
    ) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let scale = (window_w.min(window_h)) / 600.0 * ui_scale;
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        // Dim overlay background.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: [0.0, 0.0, 0.0, 0.7],
        });

        // Panel dimensions.
        let panel_w = (520.0 * scale).min(window_w * 0.90);
        let row_h = (40.0 * scale).max(26.0);
        let desc_h = (18.0 * scale).max(12.0);
        let row_gap = (10.0 * scale).max(4.0);
        let title_h = (48.0 * scale).max(28.0);
        let diagram_h = (80.0 * scale).max(50.0);
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
            color: [0.08, 0.08, 0.14, 0.95],
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
            color: [0.3, 0.45, 0.7, 0.8],
        });
        // Re-draw panel on top of border.
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: [0.08, 0.08, 0.14, 0.95],
        });

        // Title.
        labels.push(TextLabel {
            rect: [panel_x, panel_y + row_gap, panel_w, title_h],
            text: "Cascade Tuning".into(),
            color: [1.0, 0.95, 0.7, 1.0],
            ..Default::default()
        });

        let mut cursor_y = panel_y + row_gap + title_h + row_gap;

        // Timing diagram.
        let diag_pad = 12.0 * scale;
        instances.push(GpuInstance {
            rect: [
                panel_x + diag_pad,
                cursor_y,
                panel_w - diag_pad * 2.0,
                diagram_h,
            ],
            color: [0.06, 0.06, 0.10, 0.9],
        });
        // Draw timeline segments proportional to actual values.
        let total_ms =
            self.tuning.base_hold_ms + self.tuning.step_hold_ms * 2 + self.tuning.total_hold_ms;
        let bar_x = panel_x + diag_pad + 8.0 * scale;
        let bar_w = panel_w - diag_pad * 2.0 - 16.0 * scale;
        let bar_h = (16.0 * scale).max(10.0);
        let bar_y = cursor_y + diagram_h * 0.35;
        let colors: [[f32; 4]; 4] = [
            [0.35, 0.65, 0.90, 0.9], // base hold (blue)
            [0.55, 0.80, 0.45, 0.9], // step 1 (green)
            [0.45, 0.70, 0.35, 0.9], // step 2 (green darker)
            [0.90, 0.75, 0.30, 0.9], // total hold (gold)
        ];
        let segments: [u64; 4] = [
            self.tuning.base_hold_ms,
            self.tuning.step_hold_ms,
            self.tuning.step_hold_ms,
            self.tuning.total_hold_ms,
        ];
        let seg_labels = ["Base", "Step", "Step", "Total"];
        let mut seg_x = bar_x;
        for (i, &ms) in segments.iter().enumerate() {
            let seg_w = bar_w * (ms as f32 / total_ms as f32);
            instances.push(GpuInstance {
                rect: [seg_x, bar_y, seg_w, bar_h],
                color: colors[i],
            });
            // Segment label (centered in segment).
            if seg_w > 20.0 {
                labels.push(TextLabel {
                    rect: [seg_x, bar_y, seg_w, bar_h],
                    text: seg_labels[i].to_string(),
                    color: [0.0, 0.0, 0.0, 0.9],
                    ..Default::default()
                });
            }
            seg_x += seg_w;
        }
        // Diagram title.
        labels.push(TextLabel {
            rect: [
                panel_x + diag_pad,
                cursor_y + 2.0,
                panel_w - diag_pad * 2.0,
                diagram_h * 0.28,
            ],
            text: "Timeline: Base > Steps (x N) > Total".into(),
            color: [0.6, 0.6, 0.7, 0.8],
            ..Default::default()
        });
        // Tick duration annotation.
        let tick_label_y = bar_y + bar_h + 4.0 * scale;
        labels.push(TextLabel {
            rect: [
                panel_x + diag_pad,
                tick_label_y,
                panel_w - diag_pad * 2.0,
                diagram_h * 0.25,
            ],
            text: format!(
                "Score counter ticks over {}ms per phase",
                self.tuning.tick_duration_ms
            ),
            color: [0.5, 0.5, 0.6, 0.7],
            ..Default::default()
        });

        cursor_y += diagram_h + row_gap;

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
                "Discard Speed",
                "How long discarded tiles float away",
                self.tuning.depart_lifetime_ms,
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
            (
                "Wind Delay",
                "Pause after deal before the smoke gust + candle dim",
                self.tuning.wind_delay_ms,
            ),
            (
                "Wind Duration",
                "Length of the post-deal smoke gust + candle dim envelope",
                self.tuning.wind_duration_ms,
            ),
        ];

        for (i, (name, desc, value)) in rows.iter().enumerate() {
            let row_y = cursor_y + i as f32 * row_total_h;
            let is_focused = self.cursor == i;

            // Row background.
            let bg = if is_focused {
                [0.20, 0.32, 0.50, 0.90]
            } else {
                [0.12, 0.15, 0.24, 0.75]
            };
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h + desc_h],
                color: bg,
            });

            // Label.
            let tc = if is_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.6, 0.6, 0.7, 0.9]
            };
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
                color: [0.45, 0.45, 0.55, 0.7],
                ..Default::default()
            });

            // Slider track.
            let track_x = panel_x + label_w;
            let track_h = (8.0 * scale).max(4.0);
            let track_y = row_y + (row_h - track_h) * 0.5;
            instances.push(GpuInstance {
                rect: [track_x, track_y, slider_w, track_h],
                color: [0.08, 0.08, 0.14, 1.0],
            });

            // Slider fill.
            let t = (*value as f32 - TUNING_MIN_MS as f32) / (TUNING_MAX_MS - TUNING_MIN_MS) as f32;
            let fill_w = slider_w * t.clamp(0.0, 1.0);
            let fill_color = if is_focused {
                [0.35, 0.65, 0.90, 1.0]
            } else {
                [0.22, 0.42, 0.62, 0.85]
            };
            instances.push(GpuInstance {
                rect: [track_x, track_y, fill_w, track_h],
                color: fill_color,
            });

            // Knob.
            let knob_size = track_h * 2.5;
            let knob_x = track_x + fill_w - knob_size * 0.5;
            let knob_y = track_y + (track_h - knob_size) * 0.5;
            instances.push(GpuInstance {
                rect: [knob_x, knob_y, knob_size, knob_size],
                color: if is_focused {
                    [0.9, 0.9, 1.0, 1.0]
                } else {
                    [0.6, 0.6, 0.7, 0.9]
                },
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
        let is_focused = self.cursor == TUNING_ROW_COUNT - 1;
        let bg = if is_focused {
            [0.25, 0.45, 0.30, 0.95]
        } else {
            [0.15, 0.20, 0.18, 0.85]
        };
        instances.push(GpuInstance {
            rect: [panel_x + 4.0, export_y, panel_w - 8.0, row_h],
            color: bg,
        });
        labels.push(TextLabel {
            rect: [panel_x, export_y, panel_w, row_h],
            text: "Export as JSON".into(),
            color: if is_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.6, 0.6, 0.7, 0.9]
            },
            ..Default::default()
        });

        // Hint.
        let hint_y = export_y + row_h + row_gap;
        labels.push(TextLabel {
            rect: [panel_x, hint_y, panel_w, row_h * 0.6],
            text: "Left/Right: adjust   Esc: close".into(),
            color: [0.4, 0.4, 0.5, 0.6],
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
}

impl SfxTestOverlay {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            row_rects: Vec::new(),
        }
    }

    /// Returns `true` if the overlay should close.
    ///
    /// `mouse` is `Some((x, y, clicked))` when cursor position is known;
    /// `clicked` is true on the frame a left-button press landed.
    pub fn update(
        &mut self,
        actions: &[UiAction],
        audio: &audio::AudioManager,
        mouse: Option<(f32, f32, bool)>,
    ) -> bool {
        let count = audio::all_sfx_ids().len();

        // Mouse hover → move cursor; click → play.
        if let Some((mx, my, clicked)) = mouse {
            for (i, r) in self.row_rects.iter().enumerate() {
                if mx >= r[0] && mx <= r[0] + r[2] && my >= r[1] && my <= r[1] + r[3] {
                    self.cursor = i;
                    if clicked {
                        if let Some(&id) = audio::all_sfx_ids().get(i) {
                            audio.play_sfx(id);
                        }
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

    pub fn draw(
        &mut self,
        window_w: f32,
        window_h: f32,
        ui_scale: f32,
    ) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let scale = (window_w.min(window_h)) / 600.0 * ui_scale;
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        // Dim overlay background.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: [0.0, 0.0, 0.0, 0.7],
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
            color: [0.55, 0.45, 0.20, 0.85],
        });
        // Panel background (Midnight Gold cool indigo).
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: [0.06, 0.07, 0.14, 0.97],
        });

        // Title.
        labels.push(TextLabel {
            rect: [panel_x, panel_y + pad, panel_w, title_h],
            text: "Sound Effects Test".into(),
            color: [1.0, 0.85, 0.45, 1.0],
            ..Default::default()
        });

        // Rows — also cache hit-test rects for mouse support.
        self.row_rects.clear();
        let rows_y0 = panel_y + pad + title_h + pad;
        for (i, &id) in ids.iter().enumerate() {
            let row_y = rows_y0 + i as f32 * (row_h + row_gap);
            self.row_rects
                .push([panel_x + 4.0, row_y, panel_w - 8.0, row_h]);
            let is_focused = self.cursor == i;

            let bg = if is_focused {
                [0.30, 0.26, 0.50, 0.95]
            } else {
                [0.10, 0.12, 0.20, 0.80]
            };
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h],
                color: bg,
            });

            let tc = if is_focused {
                [1.0, 0.92, 0.60, 1.0]
            } else {
                [0.65, 0.65, 0.78, 0.95]
            };
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
            color: [0.55, 0.55, 0.65, 0.75],
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
        }
    }

    pub fn to_camera_params(&self) -> CameraParams {
        CameraParams {
            eye: self.eye,
            target: self.target,
            up: self.up,
            fovy_deg: self.fovy_deg,
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
    pub fn update(&mut self, actions: &[UiAction], window_h: f32) -> bool {
        self.last_window_h = window_h;
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
                                log::error!("[Debug] Clipboard write failed: {e}");
                            } else {
                                log::info!("[Debug] Camera params copied to clipboard");
                            }
                        }
                        Err(e) => log::error!("[Debug] Could not open clipboard: {e}"),
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

    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        ui_scale: f32,
    ) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let scale = (window_w.min(window_h)) / 600.0 * ui_scale;
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        // Dim background.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: [0.0, 0.0, 0.0, 0.55],
        });

        let row_h = (36.0 * scale).max(24.0);
        let row_gap = (6.0 * scale).max(3.0);
        let title_h = (48.0 * scale).max(28.0);
        let hint_h = (22.0 * scale).max(14.0);
        let pad = (16.0 * scale).max(8.0);
        let rows = self.rows();

        let panel_w = (480.0 * scale).min(window_w * 0.85);
        let panel_h =
            pad + title_h + pad + (rows.len() as f32) * (row_h + row_gap) + pad + hint_h + pad;
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
            color: [0.55, 0.45, 0.20, 0.85],
        });
        // Panel background.
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: [0.06, 0.07, 0.14, 0.97],
        });

        // Title.
        labels.push(TextLabel {
            rect: [panel_x, panel_y + pad, panel_w, title_h],
            text: "Camera Debug".into(),
            color: [1.0, 0.85, 0.45, 1.0],
            ..Default::default()
        });

        // Rows.
        let rows_y0 = panel_y + pad + title_h + pad;
        for (i, (name, value)) in rows.iter().enumerate() {
            let row_y = rows_y0 + i as f32 * (row_h + row_gap);
            let is_focused = self.cursor == i;

            let bg = if is_focused {
                [0.30, 0.26, 0.50, 0.95]
            } else {
                [0.10, 0.12, 0.20, 0.80]
            };
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h],
                color: bg,
            });

            let tc = if is_focused {
                [1.0, 0.92, 0.60, 1.0]
            } else {
                [0.65, 0.65, 0.78, 0.95]
            };
            // Label (left).
            let name_w = panel_w * 0.45;
            labels.push(TextLabel {
                rect: [panel_x + 12.0 * scale, row_y, name_w, row_h],
                text: name.to_string(),
                color: tc,
                ..Default::default()
            });
            // Value (right).
            labels.push(TextLabel {
                rect: [
                    panel_x + 12.0 * scale + name_w,
                    row_y,
                    panel_w - name_w - 24.0 * scale,
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
            text: "Up/Down: select   Left/Right: adjust   Enter: copy   Esc: close".into(),
            color: [0.55, 0.55, 0.65, 0.75],
            ..Default::default()
        });

        (instances, labels)
    }
}
