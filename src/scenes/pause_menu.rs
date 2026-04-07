//! Shared pause menu overlay used by gameplay, shop, and blind-selection scenes.

use crate::game::run::RunState;
use crate::render::theme::{ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget;

use super::shop::ShopScene;
use super::start_screen::StartScreenScene;
use super::{ButtonDef, Scene, SceneTransition};

/// Pause menu item indices.
const PAUSE_RESUME: usize = 0;
const PAUSE_RESTART: usize = 1;
const PAUSE_MAIN_MENU: usize = 2;
const PAUSE_EXIT: usize = 3;
const PAUSE_COUNT: usize = 4;

/// Result of processing pause menu input for the current frame.
pub enum PauseUpdate {
    /// Stay paused, no scene transition.
    StayPaused,
    /// Resume the current scene (unpause).
    Resume,
    /// Transition to another scene.
    Transition(SceneTransition),
    /// Quit the application.
    Quit,
}

/// Reusable pause menu state.
pub struct PauseMenu {
    pub paused: bool,
    cursor: usize,
}

impl PauseMenu {
    pub fn new() -> Self {
        Self {
            paused: false,
            cursor: PAUSE_RESUME,
        }
    }

    /// Open the pause menu.
    pub fn open(&mut self) {
        self.paused = true;
        self.cursor = PAUSE_RESUME;
    }

    /// Process actions while paused. Returns what the caller should do.
    /// `cursor_pos` and `(window_w, window_h)` enable mouse hover-to-focus.
    pub fn update(
        &mut self,
        actions: &[UiAction],
        run: &mut RunState,
        cursor_pos: (f32, f32),
        window_w: f32,
        window_h: f32,
    ) -> PauseUpdate {
        // Mouse hover → focus the pause-menu button under the pointer.
        // Layout must mirror draw().
        let scale = (window_w.min(window_h)) / 600.0;
        let btn_w = (200.0 * scale).min(window_w * 0.5);
        let btn_h = (38.0 * scale).max(24.0);
        let btn_gap = (12.0 * scale).max(6.0);
        let total_menu_h = PAUSE_COUNT as f32 * btn_h + (PAUSE_COUNT as f32 - 1.0) * btn_gap;
        let title_h = (48.0 * scale).max(28.0);
        let title_gap = (20.0 * scale).max(10.0);
        let block_h = title_h + title_gap + total_menu_h;
        let start_y = (window_h - block_h) * 0.5;
        let menu_start_y = start_y + title_h + title_gap;
        let btn_x = (window_w - btn_w) * 0.5;
        let (cx, cy) = cursor_pos;
        for i in 0..PAUSE_COUNT {
            let by = menu_start_y + i as f32 * (btn_h + btn_gap);
            if cx >= btn_x && cx <= btn_x + btn_w && cy >= by && cy <= by + btn_h {
                self.cursor = i;
                break;
            }
        }

        for a in actions {
            match a {
                UiAction::Pause | UiAction::Cancel => {
                    self.paused = false;
                    return PauseUpdate::Resume;
                }
                UiAction::FocusDown | UiAction::FocusNext => {
                    self.cursor = (self.cursor + 1) % PAUSE_COUNT;
                }
                UiAction::FocusUp | UiAction::FocusPrev => {
                    self.cursor = (self.cursor + PAUSE_COUNT - 1) % PAUSE_COUNT;
                }
                // Mouse clicks land here too — hover-focus above already
                // pointed self.cursor at the hovered button.
                UiAction::Confirm | UiAction::CommitDiscard => return self.activate(run),
                _ => {}
            }
        }
        PauseUpdate::StayPaused
    }

    fn activate(&mut self, run: &mut RunState) -> PauseUpdate {
        match self.cursor {
            PAUSE_RESUME => {
                self.paused = false;
                PauseUpdate::Resume
            }
            PAUSE_RESTART => self.do_restart(run),
            PAUSE_MAIN_MENU => {
                PauseUpdate::Transition(Some(Scene::StartScreen(StartScreenScene::new())))
            }
            PAUSE_EXIT => PauseUpdate::Quit,
            _ => PauseUpdate::StayPaused,
        }
    }

    fn do_restart(&mut self, run: &mut RunState) -> PauseUpdate {
        *run = RunState::new_demo();
        PauseUpdate::Transition(Some(Scene::Shop(ShopScene::new(
            run.run_number,
            &run.relics,
        ))))
    }

    /// Append pause-overlay draw elements to the given vectors.
    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        scale: f32,
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        buttons: &mut Vec<ButtonDef>,
    ) {
        if !self.paused {
            return;
        }

        // Dim background — Midnight Gold theme: cool deep indigo, not pure black.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: color::alpha(color::OBSIDIAN, 0.78),
        });

        let menu_labels = ["Resume", "Restart", "Main Menu", "Exit"];
        let btn_w = (220.0 * scale).min(window_w * 0.55);
        let btn_h = (44.0 * scale).max(28.0);
        let btn_gap = (12.0 * scale).max(6.0);
        let total_menu_h = PAUSE_COUNT as f32 * btn_h + (PAUSE_COUNT as f32 - 1.0) * btn_gap;
        let title_h = typography::size(typography::TITLE, window_h);
        let title_gap = (24.0 * scale).max(10.0);
        let block_h = title_h + title_gap + total_menu_h;
        let start_y = (window_h - block_h) * 0.5;
        let btn_x = (window_w - btn_w) * 0.5;

        // Title — gold serif, centered.
        text_labels.push(TextLabel {
            rect: [0.0, start_y, window_w, title_h],
            text: "PAUSED".into(),
            color: color::CHAMPAGNE,
        });

        let menu_start_y = start_y + title_h + title_gap;

        for (i, label) in menu_labels.iter().enumerate() {
            let by = menu_start_y + i as f32 * (btn_h + btn_gap);
            let is_focused = i == self.cursor;

            let variant = match i {
                PAUSE_RESUME => ButtonVariant::Primary,
                PAUSE_EXIT => ButtonVariant::Danger,
                _ => ButtonVariant::Default,
            };
            let state = if is_focused {
                ButtonState::Hover
            } else {
                ButtonState::Rest
            };

            // Click → Confirm; hover-focus in update() ensures self.cursor
            // already points at this button by the time the action is read.
            widget::push_button(
                instances,
                text_labels,
                buttons,
                [btn_x, by, btn_w, btn_h],
                label,
                variant,
                state,
                UiAction::Confirm,
            );
        }
    }
}
