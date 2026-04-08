//! Options scene — volume sliders and audio settings.

use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::start_screen::StartScreenScene;
use super::{
    ButtonDef, DrawCtx, Scene, SceneBehavior, SceneDrawOutput, SceneTransition, UpdateCtx,
};

/// Volume adjustment step per input press.
const VOL_STEP: f32 = 0.05;
/// Gamma adjustment step per input press.
const GAMMA_STEP: f32 = 0.05;

/// Logical row identity. Adding/reordering options means: add a variant, add
/// it to `ROWS`, add a match arm in `apply_action`. No const indices to
/// renumber, no parallel arrays to keep in sync, and the compiler enforces
/// exhaustive handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Row {
    Master,
    Music,
    Sfx,
    Gamma,
    SfxToggle,
    Smoke,
    SmokeDetail,
    Tile,
    Shadows,
    Ssr,
    Back,
}

const ROWS: &[Row] = &[
    Row::Master,
    Row::Music,
    Row::Sfx,
    Row::Gamma,
    Row::SfxToggle,
    Row::Smoke,
    Row::SmokeDetail,
    Row::Tile,
    Row::Shadows,
    Row::Ssr,
    Row::Back,
];

impl Row {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }

    fn from_id(id: FocusId) -> Option<Row> {
        ROWS.iter().copied().find(|r| r.id() == id)
    }

    fn is_slider(self) -> bool {
        matches!(self, Row::Master | Row::Music | Row::Sfx | Row::Gamma)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OptAction {
    /// Click on a row — handled by `apply_click`.
    Click(Row),
}

pub struct OptionsScene {
    tree: TreeState,
    /// Local copy of settings; written back on change and scene exit.
    pub master_volume: f32,
    pub sfx_volume: f32,
    pub music_volume: f32,
    pub sfx_enabled: bool,
    pub smoke_intensity: crate::persistence::SmokeIntensity,
    pub smoke_detail: crate::persistence::SmokeDetail,
    pub tile_preset: crate::persistence::TilePreset,
    pub gamma: f32,
    pub shadows_enabled: bool,
    pub ssr_enabled: bool,
}

impl OptionsScene {
    pub fn new() -> Self {
        let settings = crate::persistence::load_settings();
        let mut tree = TreeState::new();
        tree.set_focus(Row::Master.id());
        Self {
            tree,
            master_volume: settings.master_volume,
            sfx_volume: settings.sfx_volume,
            music_volume: settings.music_volume,
            sfx_enabled: settings.sfx_enabled,
            smoke_intensity: settings.smoke_intensity,
            smoke_detail: settings.smoke_detail,
            tile_preset: settings.tile_preset,
            gamma: settings.gamma,
            shadows_enabled: settings.shadows_enabled,
            ssr_enabled: settings.ssr_enabled,
        }
    }

    fn save_settings(&self) {
        let mut settings = crate::persistence::load_settings();
        settings.master_volume = self.master_volume;
        settings.sfx_volume = self.sfx_volume;
        settings.music_volume = self.music_volume;
        settings.sfx_enabled = self.sfx_enabled;
        settings.smoke_intensity = self.smoke_intensity;
        settings.smoke_detail = self.smoke_detail;
        settings.tile_preset = self.tile_preset;
        settings.gamma = self.gamma;
        settings.shadows_enabled = self.shadows_enabled;
        settings.ssr_enabled = self.ssr_enabled;
        let _ = crate::persistence::save_settings(&settings);
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

    fn focused_row(&self) -> Row {
        self.tree
            .focused()
            .and_then(Row::from_id)
            .unwrap_or(Row::Master)
    }

    /// Single source of truth for row layout — used by update() (hit-test),
    /// draw() (rendering), AND register_flat_buttons() (click registration).
    fn row_layout(window_w: f32, window_h: f32) -> RowLayout {
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
        let label_w = row_w * 0.35;
        let slider_x = row_x + label_w;
        let slider_w = row_w * 0.50;
        RowLayout {
            scale,
            title_h,
            title_y,
            row_x,
            row_w,
            row_h,
            row_gap,
            menu_start_y,
            label_w,
            slider_x,
            slider_w,
        }
    }

    fn flat_items(layout: &RowLayout) -> Vec<FlatItem<OptAction>> {
        ROWS.iter()
            .enumerate()
            .map(|(i, &row)| {
                let row_y = layout.menu_start_y + i as f32 * (layout.row_h + layout.row_gap);
                FlatItem::new(
                    row.id(),
                    [layout.row_x, row_y, layout.row_w, layout.row_h],
                    OptAction::Click(row),
                )
            })
            .collect()
    }

    /// Process one frame of input against the options menu state. Returns
    /// `true` if the user requested to close (Back / Cancel / Pause).
    ///
    /// Shared by the standalone `OptionsScene` and the in-game pause menu's
    /// embedded options overlay so both stay in sync without duplicating
    /// input logic.
    pub fn update_input(
        &mut self,
        actions: &[UiAction],
        button_clicks: &[u32],
        cursor_pos: (f32, f32),
        window_w: f32,
        window_h: f32,
    ) -> bool {
        let layout = Self::row_layout(window_w, window_h);
        let items = Self::flat_items(&layout);
        // Left/Right (FocusNext/FocusPrev) are reserved here for adjusting the
        // focused slider/cycle row — don't let the tree consume them as
        // vertical navigation, or pressing Left/Right would both move focus
        // AND change the value. Only Up/Down navigate this menu.
        let nav_actions: Vec<UiAction> = actions
            .iter()
            .copied()
            .filter(|a| !matches!(a, UiAction::FocusNext | UiAction::FocusPrev))
            .collect();
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: &nav_actions,
                button_clicks,
                cursor_pos,
                window: (window_w, window_h),
            },
        );

        // Sliders/cycles need keyboard adjustment that the generic tree doesn't
        // model — read the focused row and apply Left/Right adjustments here.
        let focused = self.focused_row();
        for a in actions {
            match a {
                UiAction::FocusNext => {
                    if focused.is_slider() {
                        self.adjust_slider(focused, 1.0);
                    } else if focused == Row::Smoke {
                        self.smoke_intensity = self.smoke_intensity.next();
                        self.save_settings();
                    } else if focused == Row::SmokeDetail {
                        self.smoke_detail = self.smoke_detail.next();
                        self.save_settings();
                    } else if focused == Row::Tile {
                        self.tile_preset = self.tile_preset.next();
                        self.save_settings();
                    } else if focused == Row::Shadows {
                        self.shadows_enabled = !self.shadows_enabled;
                        self.save_settings();
                    } else if focused == Row::Ssr {
                        self.ssr_enabled = !self.ssr_enabled;
                        self.save_settings();
                    }
                }
                UiAction::FocusPrev => {
                    if focused.is_slider() {
                        self.adjust_slider(focused, -1.0);
                    } else if focused == Row::Smoke {
                        self.smoke_intensity = self.smoke_intensity.prev();
                        self.save_settings();
                    } else if focused == Row::SmokeDetail {
                        self.smoke_detail = self.smoke_detail.prev();
                        self.save_settings();
                    } else if focused == Row::Tile {
                        self.tile_preset = self.tile_preset.prev();
                        self.save_settings();
                    } else if focused == Row::Shadows {
                        self.shadows_enabled = !self.shadows_enabled;
                        self.save_settings();
                    } else if focused == Row::Ssr {
                        self.ssr_enabled = !self.ssr_enabled;
                        self.save_settings();
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    self.save_settings();
                    return true;
                }
                _ => {}
            }
        }

        if let Some(OptAction::Click(row)) = action {
            return self.apply_click(row, &layout, cursor_pos);
        }
        false
    }

    /// Apply a click on a row. Returns true if the scene should close (Back).
    fn apply_click(&mut self, row: Row, layout: &RowLayout, cursor_pos: (f32, f32)) -> bool {
        match row {
            Row::Master | Row::Music | Row::Sfx | Row::Gamma => {
                // Click-to-position: if the click is within the slider track,
                // set the slider value to the proportional position.
                let cx = cursor_pos.0;
                if cx >= layout.slider_x && cx <= layout.slider_x + layout.slider_w {
                    let ratio = (cx - layout.slider_x) / layout.slider_w;
                    let (lo, hi, _) = Self::slider_range(row);
                    self.store_slider(row, lo + ratio * (hi - lo));
                    self.save_settings();
                }
            }
            Row::SfxToggle => {
                self.sfx_enabled = !self.sfx_enabled;
                self.save_settings();
            }
            Row::Smoke => {
                self.smoke_intensity = self.smoke_intensity.next();
                self.save_settings();
            }
            Row::SmokeDetail => {
                self.smoke_detail = self.smoke_detail.next();
                self.save_settings();
            }
            Row::Tile => {
                self.tile_preset = self.tile_preset.next();
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
            Row::Back => {
                self.save_settings();
                return true;
            }
        }
        false
    }

    /// Build the options menu UI elements into the supplied buffers. Shared
    /// between the standalone scene and the in-game pause-menu overlay.
    /// Caller is responsible for any background dimming behind the menu.
    pub fn draw_overlay(
        &self,
        w: f32,
        h: f32,
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        buttons: &mut Vec<ButtonDef>,
    ) {
        let layout = Self::row_layout(w, h);
        let track_h = (8.0 * layout.scale).max(4.0);
        let pct_x = layout.slider_x + layout.slider_w + layout.row_w * 0.02;
        let pct_w = layout.row_w * 0.13;
        let focused = self.focused_row();

        // Title.
        text_labels.push(TextLabel {
            rect: [0.0, layout.title_y, w, layout.title_h],
            text: "Options".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // Render each row by its semantic identity. Adding a row only
        // requires extending the match here and the ROWS list at the top.
        let items = Self::flat_items(&layout);
        for (i, &row) in ROWS.iter().enumerate() {
            let rect @ [rx, ry, rw, rh] = items[i].rect;
            let _ = rect;
            let is_focused = row == focused;
            self.draw_row(
                instances,
                text_labels,
                row,
                [rx, ry, rw, rh],
                is_focused,
                track_h,
                pct_x,
                pct_w,
            );
        }

        // Single hit-target list shared with update() — single source of truth.
        self.tree.register_flat_buttons(&items, buttons);

        // Hint text at the bottom.
        let last_y = layout.menu_start_y
            + (ROWS.len() - 1) as f32 * (layout.row_h + layout.row_gap)
            + layout.row_h;
        let hint_h = (20.0 * layout.scale).max(14.0);
        let hint_y = last_y + layout.row_gap * 2.0;
        text_labels.push(TextLabel {
            rect: [0.0, hint_y, w, hint_h],
            text: "Up/Down: navigate   Left/Right: adjust slider   Space: toggle/select".into(),
            color: color::SLATE,
            ..Default::default()
        });
    }

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
            Row::Master | Row::Music | Row::Sfx | Row::Gamma => {
                let value = self.slider_value(row).unwrap_or(0.0);
                let (lo, hi, _) = Self::slider_range(row);
                let fill_ratio = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
                let label = match row {
                    Row::Master => "Master Volume",
                    Row::Music => "Music Volume",
                    Row::Sfx => "SFX Volume",
                    Row::Gamma => "Gamma",
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
                let readout = if matches!(row, Row::Gamma) {
                    format!("{:.2}", value)
                } else {
                    format!("{}%", (value * 100.0).round() as u32)
                };
                text_labels.push(TextLabel {
                    rect: [pct_x, row_y, pct_w, row_h],
                    text: readout,
                    color: text_color,
                    ..Default::default()
                });
            }
            Row::SfxToggle => {
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
                text_labels.push(TextLabel {
                    rect: [row_x, row_y, row_w, row_h],
                    text: format!(
                        "Sound Effects: {}",
                        if self.sfx_enabled { "ON" } else { "OFF" }
                    ),
                    color: text_color,
                    ..Default::default()
                });
            }
            Row::Smoke => {
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
                text_labels.push(TextLabel {
                    rect: [row_x, row_y, row_w, row_h],
                    text: format!("Smoke: {}", self.smoke_intensity.label()),
                    color: text_color,
                    ..Default::default()
                });
            }
            Row::SmokeDetail => {
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
                text_labels.push(TextLabel {
                    rect: [row_x, row_y, row_w, row_h],
                    text: format!("Smoke Detail: {}", self.smoke_detail.label()),
                    color: text_color,
                    ..Default::default()
                });
            }
            Row::Tile => {
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
                text_labels.push(TextLabel {
                    rect: [row_x, row_y, row_w, row_h],
                    text: format!("Tile Style: {}", self.tile_preset.label()),
                    color: text_color,
                    ..Default::default()
                });
            }
            Row::Shadows => {
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
                text_labels.push(TextLabel {
                    rect: [row_x, row_y, row_w, row_h],
                    text: format!(
                        "Shadows: {}",
                        if self.shadows_enabled { "ON" } else { "OFF" }
                    ),
                    color: text_color,
                    ..Default::default()
                });
            }
            Row::Ssr => {
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
                text_labels.push(TextLabel {
                    rect: [row_x, row_y, row_w, row_h],
                    text: format!(
                        "Reflections: {}",
                        if self.ssr_enabled { "ON" } else { "OFF" }
                    ),
                    color: text_color,
                    ..Default::default()
                });
            }
            Row::Back => {
                let bg_color = if is_focused {
                    color::TWILIGHT
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
                text_labels.push(TextLabel {
                    rect: [row_x, row_y, row_w, row_h],
                    text: "Back".into(),
                    color: text_color,
                    ..Default::default()
                });
            }
        }
    }
}

/// Cached layout numbers shared across `update_input` and `draw_overlay`.
struct RowLayout {
    scale: f32,
    title_h: f32,
    title_y: f32,
    row_x: f32,
    row_w: f32,
    row_h: f32,
    row_gap: f32,
    menu_start_y: f32,
    #[allow(dead_code)]
    label_w: f32,
    slider_x: f32,
    slider_w: f32,
}

impl SceneBehavior for OptionsScene {
    /// Standalone scene update — calls the shared input handler and translates
    /// "back pressed" into a transition to the start screen.
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if self.update_input(
            ctx.actions,
            ctx.button_clicks,
            ctx.cursor_pos,
            ctx.layout.window_w,
            ctx.layout.window_h,
        ) {
            return Some(Scene::StartScreen(StartScreenScene::new()));
        }
        None
    }

    fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        }];
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();

        self.draw_overlay(w, h, &mut instances, &mut text_labels, &mut buttons);

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
            relic_placements: vec![],
            draw_table: false,
            wind_gusts: Vec::new(),
        }
    }
}
