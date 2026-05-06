//! Debug overlay panels: visibility toggles, cascade tuning, SFX test,
//! camera parameter editor.  All are App-owned modal overlays that
//! intercept input while open.

use crate::audio;
use crate::game::cascade::CascadeTuning;
use crate::game::volumetric_tuning::VolumetricTuning;
use crate::render::draw_cmd::CameraParams;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

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
        audio: &mut audio::AudioManager,
        mouse: Option<(f32, f32, bool)>,
    ) -> bool {
        let count = audio::all_sfx_ids().len();

        // Mouse hover → move cursor; click → play.
        if let Some((mx, my, clicked)) = mouse {
            for (i, r) in self.row_rects.iter().enumerate() {
                if mx >= r[0] && mx <= r[0] + r[2] && my >= r[1] && my <= r[1] + r[3] {
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
        // Panel background (theme WALNUT_DEEP — dark walnut).
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
            color: [0.55, 0.55, 0.65, 0.75],
            ..Default::default()
        });

        (instances, labels)
    }
}

// ── Shop environment + lighting debug (height scale + `shop_glb` tunables) ─

const SHOP_ENV_DEBUG_ROW_META: [(&'static str, f32, f32, f32); 8] = [
    ("Height scale", 0.001, 40.0, 0.005),
    ("glTF light intensity", 0.0, 40.0, 0.0025),
    ("Linear exposure", 0.001, 40.0, 0.0025),
    ("Ambient scale", 0.0, 10.0, 0.0025),
    ("Lit-mesh glTF scale", 0.0, 20.0, 0.005),
    ("Candle tint R", 0.0, 15.0, 0.0025),
    ("Candle tint G", 0.0, 15.0, 0.0025),
    ("Candle tint B", 0.0, 15.0, 0.0025),
];

#[derive(Clone, Copy)]
struct ShopEnvDebugLayout {
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

impl ShopEnvDebugLayout {
    fn compute(window_w: f32, window_h: f32, ui_scale: f32, row_count: usize) -> Self {
        let scale = (window_w.min(window_h)) / 600.0 * ui_scale;
        let row_h = (22.0 * scale).max(16.0);
        let row_gap = (3.0 * scale).max(2.0);
        let title_h = (24.0 * scale).max(16.0);
        let pad = (8.0 * scale).max(5.0);
        let margin = (10.0 * scale).max(6.0);
        let panel_w = (320.0 * scale).min(window_w * 0.44);
        let panel_x = window_w - panel_w - margin;
        let panel_y = margin;
        let rows_y0 = panel_y + pad + title_h + pad;
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
}

#[inline]
fn shop_env_point_in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx <= r.0 + r.2 && my >= r.1 && my <= r.1 + r.3
}

pub struct ShopEnvDebugOverlay {
    cursor: usize,
    pub height_scale: f32,
    pub lighting: crate::render::shop_glb::ShopEnvLightingTune,
    /// Typing buffer for the value column (numeric).
    editing: bool,
    edit_buffer: String,
    /// `Some(row)` while LMB drags that slider track.
    dragging_slider: Option<usize>,
}

impl ShopEnvDebugOverlay {
    pub fn new(
        height_scale: f32,
        lighting: crate::render::shop_glb::ShopEnvLightingTune,
    ) -> Self {
        Self {
            cursor: 0,
            height_scale,
            lighting,
            editing: false,
            edit_buffer: String::new(),
            dragging_slider: None,
        }
    }

    fn row_count(&self) -> usize {
        SHOP_ENV_DEBUG_ROW_META.len()
    }

    fn row_value(&self, row: usize) -> f32 {
        match row {
            0 => self.height_scale,
            1 => self.lighting.gltf_light_intensity_scale,
            2 => self.lighting.linear_exposure,
            3 => self.lighting.ambient_scale,
            4 => self.lighting.lit_mesh_gltf_punctual_scale,
            5 => self.lighting.candle_light_color_mul[0],
            6 => self.lighting.candle_light_color_mul[1],
            7 => self.lighting.candle_light_color_mul[2],
            _ => 0.0,
        }
    }

    fn set_row_value(&mut self, row: usize, v: f32) {
        let (_, lo, hi, _) = SHOP_ENV_DEBUG_ROW_META[row];
        let v = v.clamp(lo, hi);
        match row {
            0 => self.height_scale = v,
            1 => self.lighting.gltf_light_intensity_scale = v,
            2 => self.lighting.linear_exposure = v,
            3 => self.lighting.ambient_scale = v,
            4 => self.lighting.lit_mesh_gltf_punctual_scale = v,
            5 => self.lighting.candle_light_color_mul[0] = v,
            6 => self.lighting.candle_light_color_mul[1] = v,
            7 => self.lighting.candle_light_color_mul[2] = v,
            _ => {}
        }
    }

    fn apply_slider_mx(&mut self, row: usize, mx: f32, layout: &ShopEnvDebugLayout) {
        let (tx, _, tw, _) = layout.slider_track(row);
        let (_, min, max, _) = SHOP_ENV_DEBUG_ROW_META[row];
        let t = ((mx - tx) / tw.max(1e-6)).clamp(0.0, 1.0);
        self.set_row_value(row, min + t * (max - min));
    }

    fn adjust_row(&mut self, dir: f32) {
        if self.editing {
            return;
        }
        let row = self.cursor.min(self.row_count().saturating_sub(1));
        let (_, _, _, step) = SHOP_ENV_DEBUG_ROW_META[row];
        let v = self.row_value(row) + dir * step;
        self.set_row_value(row, v);
    }

    fn clear_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    fn begin_editing(&mut self) {
        let v = self.row_value(self.cursor);
        let mut s = format!("{:.6}", v);
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

    /// Copy `shop_glb` constant block to the clipboard.
    pub fn copy_to_clipboard(&self) {
        let l = self.lighting;
        let text = format!(
            concat!(
                "pub const SHOP_ENV_HEIGHT_SCALE: f32 = {:.6};\n",
                "pub const SHOP_ENV_LINEAR_EXPOSURE_BASE: f32 = {:.6};\n",
                "pub const SHOP_GLTF_LIGHT_INTENSITY_SCALE: f32 = {:.6};\n",
                "pub const SHOP_ENV_LINEAR_EXPOSURE: f32 = {:.6};\n",
                "pub const SHOP_ENV_AMBIENT_SCALE: f32 = {:.6};\n",
                "pub const SHOP_LIT_MESH_GLTF_PUNCTUAL_SCALE: f32 = {:.6};\n",
                "pub const SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL: [f32; 3] = ",
                "[{:.6}, {:.6}, {:.6}];",
            ),
            self.height_scale,
            crate::render::shop_glb::SHOP_ENV_LINEAR_EXPOSURE_BASE,
            l.gltf_light_intensity_scale,
            l.linear_exposure,
            l.ambient_scale,
            l.lit_mesh_gltf_punctual_scale,
            l.candle_light_color_mul[0],
            l.candle_light_color_mul[1],
            l.candle_light_color_mul[2],
        );
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Err(e) = cb.set_text(&text) {
                    log::error!("[Debug] Clipboard write failed: {e}");
                } else {
                    log::info!("[Debug] Shop env + lighting constants copied to clipboard");
                }
            }
            Err(e) => log::error!("[Debug] Could not open clipboard: {e}"),
        }
    }

    /// Keyboard while overlay is open. Returns `true` if the key was consumed
    /// (caller should skip the normal gameplay key dispatch).
    pub fn feed_key_event(&mut self, event: &KeyEvent, ctrl: bool) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return false;
        };

        if ctrl && matches!(code, KeyCode::KeyC) && !self.editing {
            self.copy_to_clipboard();
            return true;
        }

        if !self.editing {
            return false;
        }

        match code {
            KeyCode::Backspace => {
                let _ = self.edit_buffer.pop();
                true
            }
            KeyCode::Escape => {
                self.clear_edit();
                true
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                self.commit_edit();
                true
            }
            KeyCode::Digit0 | KeyCode::Numpad0 => {
                self.push_edit_char('0');
                true
            }
            KeyCode::Digit1 | KeyCode::Numpad1 => {
                self.push_edit_char('1');
                true
            }
            KeyCode::Digit2 | KeyCode::Numpad2 => {
                self.push_edit_char('2');
                true
            }
            KeyCode::Digit3 | KeyCode::Numpad3 => {
                self.push_edit_char('3');
                true
            }
            KeyCode::Digit4 | KeyCode::Numpad4 => {
                self.push_edit_char('4');
                true
            }
            KeyCode::Digit5 | KeyCode::Numpad5 => {
                self.push_edit_char('5');
                true
            }
            KeyCode::Digit6 | KeyCode::Numpad6 => {
                self.push_edit_char('6');
                true
            }
            KeyCode::Digit7 | KeyCode::Numpad7 => {
                self.push_edit_char('7');
                true
            }
            KeyCode::Digit8 | KeyCode::Numpad8 => {
                self.push_edit_char('8');
                true
            }
            KeyCode::Digit9 | KeyCode::Numpad9 => {
                self.push_edit_char('9');
                true
            }
            KeyCode::Period | KeyCode::NumpadDecimal => {
                self.push_edit_char('.');
                true
            }
            KeyCode::Minus | KeyCode::NumpadSubtract => {
                self.push_edit_char('-');
                true
            }
            _ => false,
        }
    }

    /// `mouse`: `(x, y, clicked_this_frame, left_button_held)`.
    ///
    /// Returns `true` if the overlay should close.
    pub fn update(
        &mut self,
        actions: &[UiAction],
        mouse: Option<(f32, f32, bool, bool)>,
        window_w: f32,
        window_h: f32,
        ui_scale: f32,
    ) -> bool {
        let layout = ShopEnvDebugLayout::compute(window_w, window_h, ui_scale, self.row_count());
        let n = self.row_count();

        if let Some((mx, my, clicked, held)) = mouse {
            if let Some(di) = self.dragging_slider {
                if held {
                    self.apply_slider_mx(di, mx, &layout);
                }
            }

            if (clicked || held) && self.dragging_slider.is_none() {
                for i in 0..n {
                    let track = layout.slider_track(i);
                    if shop_env_point_in_rect(mx, my, track) {
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
                    if shop_env_point_in_rect(mx, my, cell) {
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

    fn format_row_display(row: usize, v: f32) -> String {
        match row {
            5..=7 => format!("{:.3}", v),
            _ => format!("{:.4}", v),
        }
    }

    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        ui_scale: f32,
    ) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let layout = ShopEnvDebugLayout::compute(window_w, window_h, ui_scale, self.row_count());
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        let pad = (8.0 * layout.scale).max(5.0);
        let title_h = (24.0 * layout.scale).max(16.0);
        let hint_h = (13.0 * layout.scale).max(9.0);
        let panel_h = pad
            + title_h
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
            color: [0.35, 0.52, 0.28, 0.85],
        });
        instances.push(GpuInstance {
            rect: [layout.panel_x, layout.panel_y, layout.panel_w, panel_h],
            color: [0.06, 0.07, 0.14, 0.97],
        });

        labels.push(TextLabel {
            rect: [layout.panel_x, layout.panel_y + pad, layout.panel_w, title_h],
            text: "Shop Env & Lighting".into(),
            color: [0.65, 1.0, 0.55, 1.0],
            ..Default::default()
        });

        for i in 0..layout.row_count {
            let (name, min, max, _) = SHOP_ENV_DEBUG_ROW_META[i];
            let row_y = layout.rows_y0 + i as f32 * (layout.row_h + layout.row_gap);
            let is_focused = self.cursor == i;
            let v = self.row_value(i);

            let bg = if is_focused {
                [0.22, 0.38, 0.28, 0.95]
            } else {
                [0.10, 0.12, 0.20, 0.80]
            };
            instances.push(GpuInstance {
                rect: [
                    layout.panel_x + 4.0,
                    row_y,
                    layout.panel_w - 8.0,
                    layout.row_h,
                ],
                color: bg,
            });

            let tc = if is_focused {
                [0.85, 1.0, 0.75, 1.0]
            } else {
                [0.65, 0.65, 0.78, 0.95]
            };
            labels.push(TextLabel {
                rect: [
                    layout.panel_x + 6.0 * layout.scale,
                    row_y,
                    layout.label_w - 4.0 * layout.scale,
                    layout.row_h,
                ],
                text: name.into(),
                color: tc,
                ..Default::default()
            });

            let (track_x, track_y, tw, th) = layout.slider_track(i);
            instances.push(GpuInstance {
                rect: [track_x, track_y, tw, th],
                color: [0.08, 0.08, 0.14, 1.0],
            });
            let t = ((v - min) / (max - min).max(1e-8)).clamp(0.0, 1.0);
            let fill_w = tw * t;
            let fill_color = if is_focused {
                [0.35, 0.85, 0.45, 1.0]
            } else {
                [0.22, 0.55, 0.32, 0.9]
            };
            instances.push(GpuInstance {
                rect: [track_x, track_y, fill_w, th],
                color: fill_color,
            });
            let knob_size = th * 2.5;
            let knob_x = track_x + fill_w - knob_size * 0.5;
            let knob_y = track_y + (th - knob_size) * 0.5;
            instances.push(GpuInstance {
                rect: [knob_x, knob_y, knob_size, knob_size],
                color: if is_focused {
                    [0.9, 1.0, 0.85, 1.0]
                } else {
                    [0.65, 0.75, 0.68, 0.95]
                },
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
                    [0.05, 0.06, 0.10, 0.95]
                } else {
                    [0.08, 0.09, 0.13, 0.75]
                },
            });
            labels.push(TextLabel {
                rect: [vx + 2.0 * layout.scale, vy, vw - 4.0 * layout.scale, vh],
                text: value_text,
                color: [tc[0] * 0.92, tc[1] * 0.92, tc[2] * 0.92, 1.0],
                font_px: Some((layout.row_h * 0.48).max(10.0)),
                ..Default::default()
            });
        }

        let hint_y = layout.rows_y0 + layout.row_count as f32 * (layout.row_h + layout.row_gap) + pad;
        labels.push(TextLabel {
            rect: [layout.panel_x, hint_y, layout.panel_w, hint_h],
            text: "Mouse: drag slider / click value to type".into(),
            color: [0.55, 0.55, 0.65, 0.75],
            ..Default::default()
        });
        labels.push(TextLabel {
            rect: [layout.panel_x, hint_y + hint_h, layout.panel_w, hint_h],
            text: "\u{2191}\u{2193} row  \u{2190}\u{2192} nudge  Enter edit/apply  Esc  Ctrl+C copy"
                .into(),
            color: [0.55, 0.55, 0.65, 0.75],
            ..Default::default()
        });

        (instances, labels)
    }
}


// ── Volumetric tuning overlay ───────────────────────────────────────────
//
// Mountain-haze / fog-wall knobs. Edits apply via `WgpuRenderer::set_haze_tuning`
// on the next frame.

pub const VOL_ROW_COUNT: usize = 8; // 6 sliders + Save-as-Default + Reset
const VOL_SLIDER_ROWS: usize = VOL_ROW_COUNT - 2;
const VOL_SAVE_ROW: usize = VOL_ROW_COUNT - 2;
const VOL_RESET_ROW: usize = VOL_ROW_COUNT - 1;

// (label, min, max, step). Order must match the `match cursor` arms in
// `adjust` and `slider_value` below — add sliders there too when this
// table grows.
const VOL_SLIDER_META: [(&str, f32, f32, f32); VOL_SLIDER_ROWS] = [
    ("Haze Density", 0.0, 3.0, 0.05),
    ("Haze Color R", 0.0, 0.5, 0.005),
    ("Haze Color G", 0.0, 0.5, 0.005),
    ("Haze Color B", 0.0, 0.5, 0.005),
    ("Haze Horizon Y", 0.0, 1.0, 0.01),
    ("Haze Drift Speed", 0.0, 3.0, 0.05),
];

pub struct VolumetricDebugOverlay {
    cursor: usize,
    pub tuning: VolumetricTuning,
}

pub enum VolumetricDebugResult {
    Stay,
    Close,
    Reset,
    SaveAsDefault,
}

impl VolumetricDebugOverlay {
    pub fn new(tuning: &VolumetricTuning) -> Self {
        Self {
            cursor: 0,
            tuning: *tuning,
        }
    }

    pub fn update(&mut self, actions: &[UiAction]) -> VolumetricDebugResult {
        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % VOL_ROW_COUNT;
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + VOL_ROW_COUNT - 1) % VOL_ROW_COUNT;
                }
                UiAction::FocusNext | UiAction::NavigateHudNext => {
                    self.adjust(1.0);
                }
                UiAction::FocusPrev | UiAction::NavigateHudPrev => {
                    self.adjust(-1.0);
                }
                UiAction::Confirm => {
                    if self.cursor == VOL_SAVE_ROW {
                        return VolumetricDebugResult::SaveAsDefault;
                    } else if self.cursor == VOL_RESET_ROW {
                        return VolumetricDebugResult::Reset;
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    return VolumetricDebugResult::Close;
                }
                _ => {}
            }
        }
        VolumetricDebugResult::Stay
    }

    fn adjust(&mut self, dir: f32) {
        if self.cursor >= VOL_SLIDER_ROWS {
            return;
        }
        let (_, min, max, step) = VOL_SLIDER_META[self.cursor];
        let delta = dir * step;
        let t = &mut self.tuning;
        match self.cursor {
            0 => t.haze_density = (t.haze_density + delta).clamp(min, max),
            1 => t.haze_color_r = (t.haze_color_r + delta).clamp(min, max),
            2 => t.haze_color_g = (t.haze_color_g + delta).clamp(min, max),
            3 => t.haze_color_b = (t.haze_color_b + delta).clamp(min, max),
            4 => t.haze_horizon_y = (t.haze_horizon_y + delta).clamp(min, max),
            5 => t.haze_drift_speed = (t.haze_drift_speed + delta).clamp(min, max),
            _ => {}
        }
    }

    fn slider_value(&self, i: usize) -> f32 {
        match i {
            0 => self.tuning.haze_density,
            1 => self.tuning.haze_color_r,
            2 => self.tuning.haze_color_g,
            3 => self.tuning.haze_color_b,
            4 => self.tuning.haze_horizon_y,
            5 => self.tuning.haze_drift_speed,
            _ => 0.0,
        }
    }

    fn format_value(&self, i: usize) -> String {
        // Match precision to the slider's step so tiny increments visibly
        // change the displayed value.
        match i {
            1..=3 => format!("{:.3}", self.slider_value(i)),
            _ => format!("{:.2}", self.slider_value(i)),
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

        let margin = (10.0 * scale).max(6.0);
        let panel_w = (300.0 * scale).min(window_w * 0.40);
        let row_h = (22.0 * scale).max(16.0);
        let row_gap = (3.0 * scale).max(2.0);
        let title_h = (24.0 * scale).max(16.0);
        let hint_h = (14.0 * scale).max(10.0);
        let panel_h =
            title_h + row_gap + VOL_ROW_COUNT as f32 * (row_h + row_gap) + hint_h + row_gap * 2.0;
        let panel_x = window_w - panel_w - margin;
        let panel_y = margin;

        let border = 2.0;
        instances.push(GpuInstance {
            rect: [
                panel_x - border,
                panel_y - border,
                panel_w + border * 2.0,
                panel_h + border * 2.0,
            ],
            color: [0.3, 0.45, 0.7, 0.85],
        });
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: [0.08, 0.08, 0.14, 0.92],
        });

        labels.push(TextLabel {
            rect: [panel_x, panel_y + row_gap, panel_w, title_h],
            text: "Volumetric".into(),
            color: [1.0, 0.95, 0.7, 1.0],
            ..Default::default()
        });

        let cursor_y = panel_y + row_gap + title_h + row_gap;
        let label_w = panel_w * 0.44;
        let slider_w = panel_w * 0.32;
        let value_w = panel_w * 0.20;

        for (i, &(name, min, max, _step)) in
            VOL_SLIDER_META.iter().enumerate().take(VOL_SLIDER_ROWS)
        {
            let row_y = cursor_y + i as f32 * (row_h + row_gap);
            let is_focused = self.cursor == i;

            let bg = if is_focused {
                [0.20, 0.32, 0.50, 0.90]
            } else {
                [0.12, 0.15, 0.24, 0.75]
            };
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h],
                color: bg,
            });

            let tc = if is_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.7, 0.72, 0.82, 0.9]
            };
            labels.push(TextLabel {
                rect: [panel_x + 6.0 * scale, row_y, label_w, row_h],
                text: name.into(),
                color: tc,
                ..Default::default()
            });

            let track_x = panel_x + label_w;
            let track_h = (5.0 * scale).max(3.0);
            let track_y = row_y + (row_h - track_h) * 0.5;
            instances.push(GpuInstance {
                rect: [track_x, track_y, slider_w, track_h],
                color: [0.08, 0.08, 0.14, 1.0],
            });

            let v = self.slider_value(i);
            let t = ((v - min) / (max - min)).clamp(0.0, 1.0);
            let fill_w = slider_w * t;
            let fill_color = if is_focused {
                [0.35, 0.65, 0.90, 1.0]
            } else {
                [0.22, 0.42, 0.62, 0.85]
            };
            instances.push(GpuInstance {
                rect: [track_x, track_y, fill_w, track_h],
                color: fill_color,
            });

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

            let value_x = panel_x + label_w + slider_w + 4.0;
            labels.push(TextLabel {
                rect: [value_x, row_y, value_w, row_h],
                text: self.format_value(i),
                color: tc,
                ..Default::default()
            });
        }

        let save_y = cursor_y + VOL_SLIDER_ROWS as f32 * (row_h + row_gap);
        let save_focused = self.cursor == VOL_SAVE_ROW;
        let save_bg = if save_focused {
            [0.30, 0.50, 0.35, 0.95]
        } else {
            [0.15, 0.22, 0.18, 0.85]
        };
        instances.push(GpuInstance {
            rect: [panel_x + 4.0, save_y, panel_w - 8.0, row_h],
            color: save_bg,
        });
        labels.push(TextLabel {
            rect: [panel_x, save_y, panel_w, row_h],
            text: "Save as Default".into(),
            color: if save_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.75, 0.85, 0.78, 0.9]
            },
            ..Default::default()
        });

        let reset_y = save_y + row_h + row_gap;
        let reset_focused = self.cursor == VOL_RESET_ROW;
        let reset_bg = if reset_focused {
            [0.50, 0.30, 0.30, 0.95]
        } else {
            [0.22, 0.15, 0.18, 0.85]
        };
        instances.push(GpuInstance {
            rect: [panel_x + 4.0, reset_y, panel_w - 8.0, row_h],
            color: reset_bg,
        });
        labels.push(TextLabel {
            rect: [panel_x, reset_y, panel_w, row_h],
            text: "Reset to Defaults".into(),
            color: if reset_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.7, 0.72, 0.82, 0.9]
            },
            ..Default::default()
        });

        let hint_y = reset_y + row_h + row_gap;
        labels.push(TextLabel {
            rect: [panel_x, hint_y, panel_w, hint_h],
            text:
                "\u{2191}/\u{2193} select   \u{2190}/\u{2192} adjust   \u{23ce} confirm   Esc close"
                    .into(),
            color: [0.55, 0.6, 0.75, 0.9],
            ..Default::default()
        });

        (instances, labels)
    }
}
