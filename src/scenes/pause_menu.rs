//! Shared pause menu overlay used by gameplay, shop, and blind-selection scenes.

use crate::game::run::RunState;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::{ButtonDef, Scene, SceneTransition};
use super::shop::ShopScene;
use super::start_screen::StartScreenScene;

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
    pub fn update(&mut self, actions: &[UiAction], run: &mut RunState) -> PauseUpdate {
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
                UiAction::Confirm => return self.activate(run),
                // Mouse-click mapped actions for pause buttons.
                UiAction::CommitDiscard => {
                    self.paused = false;
                    return PauseUpdate::Resume;
                }
                UiAction::ScoreHand => return self.do_restart(run),
                UiAction::SortBySuit => {
                    return PauseUpdate::Transition(Some(Scene::StartScreen(
                        StartScreenScene::new(),
                    )));
                }
                UiAction::SortByRank => return PauseUpdate::Quit,
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
            PAUSE_MAIN_MENU => PauseUpdate::Transition(Some(Scene::StartScreen(
                StartScreenScene::new(),
            ))),
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

        // Dim background.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: [0.0, 0.0, 0.0, 0.6],
        });

        let menu_labels = ["Resume", "Restart", "Main Menu", "Exit"];
        let pause_button_actions = [
            UiAction::CommitDiscard, // Resume
            UiAction::ScoreHand,     // Restart
            UiAction::SortBySuit,    // Main Menu
            UiAction::SortByRank,    // Exit
        ];
        let btn_w = (200.0 * scale).min(window_w * 0.5);
        let btn_h = (38.0 * scale).max(24.0);
        let btn_gap = (12.0 * scale).max(6.0);
        let total_menu_h =
            PAUSE_COUNT as f32 * btn_h + (PAUSE_COUNT as f32 - 1.0) * btn_gap;
        let title_h = (48.0 * scale).max(28.0);
        let title_gap = (20.0 * scale).max(10.0);
        let block_h = title_h + title_gap + total_menu_h;
        let start_y = (window_h - block_h) * 0.5;
        let btn_x = (window_w - btn_w) * 0.5;

        // Title.
        text_labels.push(TextLabel {
            rect: [0.0, start_y, window_w, title_h],
            text: "PAUSED".into(),
            color: [1.0, 0.95, 0.7, 1.0],
        });

        let menu_start_y = start_y + title_h + title_gap;

        for (i, label) in menu_labels.iter().enumerate() {
            let by = menu_start_y + i as f32 * (btn_h + btn_gap);
            let is_focused = i == self.cursor;

            let bg_color = if is_focused {
                match i {
                    PAUSE_RESUME => [0.2, 0.55, 0.3, 0.95],
                    PAUSE_EXIT => [0.55, 0.2, 0.2, 0.95],
                    _ => [0.25, 0.4, 0.6, 0.95],
                }
            } else {
                [0.15, 0.18, 0.28, 0.85]
            };

            instances.push(GpuInstance {
                rect: [btn_x, by, btn_w, btn_h],
                color: bg_color,
            });

            let text_color = if is_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.6, 0.6, 0.7, 0.9]
            };
            text_labels.push(TextLabel {
                rect: [btn_x, by, btn_w, btn_h],
                text: label.to_string(),
                color: text_color,
            });

            buttons.push(ButtonDef {
                rect: (btn_x, by, btn_w, btn_h),
                action: pause_button_actions[i],
            });
        }
    }
}
