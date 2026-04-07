//! Options scene — volume sliders and audio settings.

use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::start_screen::StartScreenScene;
use super::{ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};

const OPT_MASTER: usize = 0;
const OPT_MUSIC: usize = 1;
const OPT_SFX: usize = 2;
const OPT_TOGGLE_SFX: usize = 3;
const OPT_SMOKE: usize = 4;
const OPT_BACK: usize = 5;
const OPT_COUNT: usize = 6;

/// Volume adjustment step per input press.
const VOL_STEP: f32 = 0.05;

pub struct OptionsScene {
    cursor: usize,
    /// Local copy of settings; written back on change and scene exit.
    pub master_volume: f32,
    pub sfx_volume: f32,
    pub music_volume: f32,
    pub sfx_enabled: bool,
    pub smoke_intensity: crate::persistence::SmokeIntensity,
    /// Cached row rects from last draw, for hover hit-testing: (x, y, w, h) per row.
    row_rects: [(f32, f32, f32, f32); OPT_COUNT],
    /// Cached slider track rect per slider row: (x, w) — only valid for first 3 rows.
    slider_tracks: [(f32, f32); 3],
}

impl OptionsScene {
    pub fn new() -> Self {
        let settings = crate::persistence::load_settings();
        Self {
            cursor: OPT_MASTER,
            master_volume: settings.master_volume,
            sfx_volume: settings.sfx_volume,
            music_volume: settings.music_volume,
            sfx_enabled: settings.sfx_enabled,
            smoke_intensity: settings.smoke_intensity,
            row_rects: [(0.0, 0.0, 0.0, 0.0); OPT_COUNT],
            slider_tracks: [(0.0, 0.0); 3],
        }
    }

    fn save_settings(&self) {
        let mut settings = crate::persistence::load_settings();
        settings.master_volume = self.master_volume;
        settings.sfx_volume = self.sfx_volume;
        settings.music_volume = self.music_volume;
        settings.sfx_enabled = self.sfx_enabled;
        settings.smoke_intensity = self.smoke_intensity;
        let _ = crate::persistence::save_settings(&settings);
    }

    /// Get a mutable reference to the volume field for the current slider row.
    fn current_volume_mut(&mut self) -> Option<&mut f32> {
        match self.cursor {
            OPT_MASTER => Some(&mut self.master_volume),
            OPT_MUSIC => Some(&mut self.music_volume),
            OPT_SFX => Some(&mut self.sfx_volume),
            _ => None,
        }
    }

    fn adjust_volume(&mut self, delta: f32) {
        if let Some(vol) = self.current_volume_mut() {
            *vol = (*vol + delta).clamp(0.0, 1.0);
            // Round to nearest step to avoid float drift.
            *vol = (*vol / VOL_STEP).round() * VOL_STEP;
            self.save_settings();
        }
    }

    /// Set the volume for a slider row by absolute position (0.0–1.0).
    fn set_volume_for_row(&mut self, row: usize, value: f32) {
        let clamped = (value / VOL_STEP).round() * VOL_STEP;
        let clamped = clamped.clamp(0.0, 1.0);
        match row {
            OPT_MASTER => self.master_volume = clamped,
            OPT_MUSIC => self.music_volume = clamped,
            OPT_SFX => self.sfx_volume = clamped,
            _ => {}
        }
    }

    /// Hit-test cursor against cached row rects; returns the row index if hovering.
    fn hover_row(&self, cursor: (f32, f32)) -> Option<usize> {
        for (i, &(rx, ry, rw, rh)) in self.row_rects.iter().enumerate() {
            if rw > 0.0
                && cursor.0 >= rx
                && cursor.0 <= rx + rw
                && cursor.1 >= ry
                && cursor.1 <= ry + rh
            {
                return Some(i);
            }
        }
        None
    }

    /// Recompute cached row rects and slider track positions from current layout.
    fn recompute_layout(&mut self, window_w: f32, window_h: f32) {
        let w = window_w;
        let h = window_h;
        let scale = (w.min(h)) / 600.0;

        let title_h = (48.0 * scale).max(28.0);
        let title_y = h * 0.08;
        let row_w = (360.0 * scale).min(w * 0.75);
        let row_h = (40.0 * scale).max(26.0);
        let row_gap = (12.0 * scale).max(6.0);
        let menu_start_y = title_y + title_h + h * 0.06;
        let row_x = (w - row_w) * 0.5;

        for i in 0..OPT_COUNT {
            let row_y = menu_start_y + i as f32 * (row_h + row_gap);
            self.row_rects[i] = (row_x, row_y, row_w, row_h);
        }

        let label_w = row_w * 0.35;
        let slider_x = row_x + label_w;
        let slider_w = row_w * 0.50;
        for i in 0..3 {
            self.slider_tracks[i] = (slider_x, slider_w);
        }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        // Recompute layout so hover and click-to-position work correctly.
        self.recompute_layout(ctx.layout.window_w, ctx.layout.window_h);

        // Update cursor highlight on mouse hover.
        if let Some(row) = self.hover_row(ctx.cursor_pos) {
            self.cursor = row;
        }

        for a in ctx.actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % OPT_COUNT;
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + OPT_COUNT - 1) % OPT_COUNT;
                }
                UiAction::FocusNext | UiAction::CommitDiscard | UiAction::NavigateHudNext => {
                    // Right / increase on slider rows; cycle on smoke; move down otherwise.
                    if self.current_volume_mut().is_some() {
                        self.adjust_volume(VOL_STEP);
                    } else if self.cursor == OPT_SMOKE {
                        self.smoke_intensity = self.smoke_intensity.next();
                        self.save_settings();
                    } else {
                        self.cursor = (self.cursor + 1) % OPT_COUNT;
                    }
                }
                UiAction::FocusPrev | UiAction::ScoreHand | UiAction::NavigateHudPrev => {
                    // Left / decrease on slider rows; cycle on smoke; move up otherwise.
                    if self.current_volume_mut().is_some() {
                        self.adjust_volume(-VOL_STEP);
                    } else if self.cursor == OPT_SMOKE {
                        self.smoke_intensity = self.smoke_intensity.prev();
                        self.save_settings();
                    } else {
                        self.cursor = (self.cursor + OPT_COUNT - 1) % OPT_COUNT;
                    }
                }
                UiAction::Confirm => match self.cursor {
                    row @ (OPT_MASTER | OPT_MUSIC | OPT_SFX) => {
                        // Click-to-position: if the click is within the slider track,
                        // set the volume to the proportional position.
                        let (track_x, track_w) = self.slider_tracks[row];
                        if track_w > 0.0 {
                            let cx = ctx.cursor_pos.0;
                            if cx >= track_x && cx <= track_x + track_w {
                                let ratio = (cx - track_x) / track_w;
                                self.set_volume_for_row(row, ratio);
                                self.save_settings();
                            }
                        }
                    }
                    OPT_TOGGLE_SFX => {
                        self.sfx_enabled = !self.sfx_enabled;
                        self.save_settings();
                    }
                    OPT_SMOKE => {
                        self.smoke_intensity = self.smoke_intensity.next();
                        self.save_settings();
                    }
                    OPT_BACK => {
                        self.save_settings();
                        return Some(Scene::StartScreen(StartScreenScene::new()));
                    }
                    _ => {}
                },
                UiAction::Cancel | UiAction::Pause => {
                    self.save_settings();
                    return Some(Scene::StartScreen(StartScreenScene::new()));
                }
                _ => {}
            }
        }
        None
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        }];
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();

        // Title.
        let title_h = (48.0 * scale).max(28.0);
        let title_y = h * 0.08;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "Options".into(),
            color: color::CHAMPAGNE,
        });

        let row_w = (360.0 * scale).min(w * 0.75);
        let row_h = (40.0 * scale).max(26.0);
        let row_gap = (12.0 * scale).max(6.0);
        let menu_start_y = title_y + title_h + h * 0.06;
        let row_x = (w - row_w) * 0.5;

        // Slider layout constants.
        let label_w = row_w * 0.35;
        let slider_x = row_x + label_w;
        let slider_w = row_w * 0.50;
        let pct_x = slider_x + slider_w + row_w * 0.02;
        let pct_w = row_w * 0.13;
        let track_h = (8.0 * scale).max(4.0);

        let slider_rows: [(usize, &str, f32); 3] = [
            (OPT_MASTER, "Master Volume", self.master_volume),
            (OPT_MUSIC, "Music Volume", self.music_volume),
            (OPT_SFX, "SFX Volume", self.sfx_volume),
        ];

        for (idx, label, value) in &slider_rows {
            let row_y = menu_start_y + *idx as f32 * (row_h + row_gap);
            let is_focused = self.cursor == *idx;

            // Row background.
            let bg_color = if is_focused {
                [0.20, 0.32, 0.50, 0.90]
            } else {
                [0.12, 0.15, 0.24, 0.75]
            };
            instances.push(GpuInstance {
                rect: [row_x, row_y, row_w, row_h],
                color: bg_color,
            });

            // Label text.
            let text_color = if is_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.6, 0.6, 0.7, 0.9]
            };
            text_labels.push(TextLabel {
                rect: [row_x + 8.0 * scale, row_y, label_w - 8.0 * scale, row_h],
                text: label.to_string(),
                color: text_color,
            });

            // Slider track (background).
            let track_y = row_y + (row_h - track_h) * 0.5;
            instances.push(GpuInstance {
                rect: [slider_x, track_y, slider_w, track_h],
                color: color::OBSIDIAN,
            });

            // Slider fill.
            let fill_w = slider_w * value;
            let fill_color = if is_focused { color::GOLD } else { color::BRASS };
            instances.push(GpuInstance {
                rect: [slider_x, track_y, fill_w, track_h],
                color: fill_color,
            });

            // Slider knob.
            let knob_size = track_h * 2.5;
            let knob_x = slider_x + fill_w - knob_size * 0.5;
            let knob_y = track_y + (track_h - knob_size) * 0.5;
            let knob_color = if is_focused { color::CHAMPAGNE } else { color::PARCHMENT };
            instances.push(GpuInstance {
                rect: [knob_x, knob_y, knob_size, knob_size],
                color: knob_color,
            });

            // Percentage text.
            let pct = (value * 100.0).round() as u32;
            text_labels.push(TextLabel {
                rect: [pct_x, row_y, pct_w, row_h],
                text: format!("{pct}%"),
                color: text_color,
            });

            // Clickable button for the whole row.
            buttons.push(ButtonDef::ui(
                (row_x, row_y, row_w, row_h),
                UiAction::Confirm,
            ));
        }

        // SFX toggle row.
        let toggle_y = menu_start_y + OPT_TOGGLE_SFX as f32 * (row_h + row_gap);
        let is_focused = self.cursor == OPT_TOGGLE_SFX;
        let bg_color = if is_focused { color::DUSK } else { color::INDIGO };
        instances.push(GpuInstance {
            rect: [row_x, toggle_y, row_w, row_h],
            color: bg_color,
        });
        let text_color = if is_focused { color::CHAMPAGNE } else { color::MIST };
        text_labels.push(TextLabel {
            rect: [row_x, toggle_y, row_w, row_h],
            text: format!(
                "Sound Effects: {}",
                if self.sfx_enabled { "ON" } else { "OFF" }
            ),
            color: text_color,
        });
        buttons.push(ButtonDef::ui(
            (row_x, toggle_y, row_w, row_h),
            UiAction::Confirm,
        ));

        // Smoke intensity row.
        let smoke_y = menu_start_y + OPT_SMOKE as f32 * (row_h + row_gap);
        let is_focused = self.cursor == OPT_SMOKE;
        let bg_color = if is_focused { color::DUSK } else { color::INDIGO };
        instances.push(GpuInstance {
            rect: [row_x, smoke_y, row_w, row_h],
            color: bg_color,
        });
        let text_color = if is_focused { color::CHAMPAGNE } else { color::MIST };
        text_labels.push(TextLabel {
            rect: [row_x, smoke_y, row_w, row_h],
            text: format!("Smoke: {}", self.smoke_intensity.label()),
            color: text_color,
        });
        buttons.push(ButtonDef::ui(
            (row_x, smoke_y, row_w, row_h),
            UiAction::Confirm,
        ));

        // Back row.
        let back_y = menu_start_y + OPT_BACK as f32 * (row_h + row_gap);
        let is_focused = self.cursor == OPT_BACK;
        let bg_color = if is_focused { color::TWILIGHT } else { color::INDIGO };
        instances.push(GpuInstance {
            rect: [row_x, back_y, row_w, row_h],
            color: bg_color,
        });
        let text_color = if is_focused { color::CHAMPAGNE } else { color::MIST };
        text_labels.push(TextLabel {
            rect: [row_x, back_y, row_w, row_h],
            text: "Back".into(),
            color: text_color,
        });
        buttons.push(ButtonDef::ui(
            (row_x, back_y, row_w, row_h),
            UiAction::Confirm,
        ));

        // Hint text at the bottom.
        let hint_h = (20.0 * scale).max(14.0);
        let hint_y = back_y + row_h + row_gap * 2.0;
        text_labels.push(TextLabel {
            rect: [0.0, hint_y, w, hint_h],
            text: "Up/Down: navigate   Left/Right: adjust slider   Space: toggle/select".into(),
            color: color::SLATE,
        });

        SceneDrawOutput {
            background: Default::default(),
            tray_instances: vec![],
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons: vec![],
            buttons,
            window_title: "Mahjuro — Options".into(),
            departing_indices: vec![],
            hint_indices: vec![],
            flame_instances: vec![],
            point_lights: vec![],
            candles: vec![],
            draw_table: false,
        }
    }
}
