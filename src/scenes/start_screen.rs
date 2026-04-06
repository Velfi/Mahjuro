//! Start screen — title screen with main menu.

use crate::game::run::RunState;
use crate::persistence;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::{ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};
use super::collection::CollectionScene;
use super::gameplay::GameplayScene;
use super::options::OptionsScene;
use super::profile_select::ProfileSelectScene;
use super::shop::ShopScene;

/// Menu items when a game is in progress.
const IP_CONTINUE: usize = 0;
const IP_NEW_GAME: usize = 1;
const IP_PROFILE: usize = 2;
const IP_COLLECTION: usize = 3;
const IP_OPTIONS: usize = 4;
const IP_QUIT: usize = 5;
const IP_COUNT: usize = 6;

/// Menu items when no game is in progress.
const NP_PLAY: usize = 0;
const NP_PROFILE: usize = 1;
const NP_COLLECTION: usize = 2;
const NP_OPTIONS: usize = 3;
const NP_QUIT: usize = 4;
const NP_COUNT: usize = 5;

pub struct StartScreenScene {
    cursor: usize,
}

impl StartScreenScene {
    pub fn new() -> Self {
        Self { cursor: 0 }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let in_progress = ctx.run.is_in_progress();
        let count = if in_progress { IP_COUNT } else { NP_COUNT };

        for a in ctx.actions {
            match a {
                UiAction::FocusDown | UiAction::FocusNext => {
                    self.cursor = (self.cursor + 1) % count;
                }
                UiAction::FocusUp | UiAction::FocusPrev => {
                    self.cursor = (self.cursor + count - 1) % count;
                }
                UiAction::Confirm => {
                    return self.activate(self.cursor, in_progress, ctx.run, ctx.quit_requested);
                }
                // Mouse-click mapped actions (direct button clicks).
                // Each button index maps to a unique action for identification.
                UiAction::CommitDiscard => {
                    return self.activate(0, in_progress, ctx.run, ctx.quit_requested);
                }
                UiAction::ScoreHand => {
                    return self.activate(1, in_progress, ctx.run, ctx.quit_requested);
                }
                UiAction::NavigateHudNext => {
                    let idx = if in_progress { IP_PROFILE } else { NP_PROFILE };
                    return self.activate(idx, in_progress, ctx.run, ctx.quit_requested);
                }
                UiAction::SortBySuit => {
                    let idx = if in_progress { IP_COLLECTION } else { NP_COLLECTION };
                    return self.activate(idx, in_progress, ctx.run, ctx.quit_requested);
                }
                UiAction::SortByRank => {
                    let idx = if in_progress { IP_OPTIONS } else { NP_OPTIONS };
                    return self.activate(idx, in_progress, ctx.run, ctx.quit_requested);
                }
                UiAction::Cancel | UiAction::Pause => {
                    *ctx.quit_requested = true;
                }
                _ => {}
            }
        }
        None
    }

    fn activate(
        &self,
        idx: usize,
        in_progress: bool,
        run: &mut RunState,
        quit_requested: &mut bool,
    ) -> SceneTransition {
        if in_progress {
            match idx {
                IP_CONTINUE => return Some(Scene::Gameplay(GameplayScene::new())),
                IP_NEW_GAME => return self.start_game(run),
                IP_PROFILE => {
                    return Some(Scene::ProfileSelect(ProfileSelectScene::from_settings()))
                }
                IP_COLLECTION => return Some(Scene::Collection(CollectionScene::new())),
                IP_OPTIONS => return Some(Scene::Options(OptionsScene::new())),
                IP_QUIT => *quit_requested = true,
                _ => {}
            }
        } else {
            match idx {
                NP_PLAY => return self.start_game(run),
                NP_PROFILE => {
                    return Some(Scene::ProfileSelect(ProfileSelectScene::from_settings()))
                }
                NP_COLLECTION => return Some(Scene::Collection(CollectionScene::new())),
                NP_OPTIONS => return Some(Scene::Options(OptionsScene::new())),
                NP_QUIT => *quit_requested = true,
                _ => {}
            }
        }
        None
    }

    fn start_game(&self, run: &mut RunState) -> SceneTransition {
        *run = RunState::new_demo();
        Some(Scene::Shop(ShopScene::new(run.run_number, &run.relics)))
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0;
        let in_progress = ctx.game_in_progress;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.04, 0.05, 0.08, 1.0],
        }];
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();

        // Title.
        let title_h = (64.0 * scale).max(32.0);
        let title_y = h * 0.08;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "M A H J U R O".into(),
            color: [1.0, 0.95, 0.7, 1.0],
        });

        // Active profile summary below title.
        let prof_y = title_y + title_h + h * 0.04;
        let prof_h = (24.0 * scale).max(16.0);
        let summaries = persistence::all_profile_summaries();
        let active = ctx.active_profile;
        let summary = &summaries[active];
        let prof_text = if summary.exists {
            format!(
                "Profile {}  —  Level {} ({} runs)",
                active + 1,
                summary.level,
                summary.runs_completed,
            )
        } else {
            format!("Profile {}  —  New", active + 1)
        };
        text_labels.push(TextLabel {
            rect: [0.0, prof_y, w, prof_h],
            text: prof_text,
            color: [0.5, 0.55, 0.65, 0.8],
        });

        // Build menu items depending on whether a game is in progress.
        let menu_labels: Vec<&str> = if in_progress {
            vec!["Continue", "New Game", "Profile", "Collection", "Options", "Quit"]
        } else {
            vec!["Play", "Profile", "Collection", "Options", "Quit"]
        };
        let count = menu_labels.len();

        // Map each button index to a unique UiAction for mouse-click identification.
        let button_actions: Vec<UiAction> = if in_progress {
            vec![
                UiAction::CommitDiscard,   // Continue
                UiAction::ScoreHand,       // New Game
                UiAction::NavigateHudNext, // Profile
                UiAction::SortBySuit,      // Collection
                UiAction::SortByRank,      // Options
                UiAction::Cancel,          // Quit
            ]
        } else {
            vec![
                UiAction::CommitDiscard,   // Play
                UiAction::NavigateHudNext, // Profile
                UiAction::SortBySuit,      // Collection
                UiAction::SortByRank,      // Options
                UiAction::Cancel,          // Quit
            ]
        };

        let quit_idx = count - 1;
        let continue_idx = 0;

        let btn_w = (200.0 * scale).min(w * 0.5);
        let btn_h = (38.0 * scale).max(24.0);
        let btn_gap = (12.0 * scale).max(6.0);
        let menu_start_y = prof_y + prof_h + h * 0.06;
        let btn_x = (w - btn_w) * 0.5;

        for (i, label) in menu_labels.iter().enumerate() {
            let btn_y = menu_start_y + i as f32 * (btn_h + btn_gap);
            let is_focused = i == self.cursor;

            let bg_color = if is_focused {
                if i == continue_idx {
                    [0.2, 0.55, 0.3, 0.95] // green for continue/play
                } else if i == quit_idx {
                    [0.55, 0.2, 0.2, 0.95] // red for quit
                } else {
                    [0.25, 0.4, 0.6, 0.95]
                }
            } else {
                [0.15, 0.18, 0.28, 0.85]
            };

            instances.push(GpuInstance {
                rect: [btn_x, btn_y, btn_w, btn_h],
                color: bg_color,
            });

            let text_color = if is_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.6, 0.6, 0.7, 0.9]
            };
            text_labels.push(TextLabel {
                rect: [btn_x, btn_y, btn_w, btn_h],
                text: label.to_string(),
                color: text_color,
            });

            buttons.push(ButtonDef {
                rect: (btn_x, btn_y, btn_w, btn_h),
                action: button_actions[i],
            });
        }

        // Hint text at bottom.
        let hint_h = (18.0 * scale).max(12.0);
        let hint_y = h - hint_h - (12.0 * scale);
        text_labels.push(TextLabel {
            rect: [0.0, hint_y, w, hint_h],
            text: "Arrow keys to navigate  |  Enter/Space to select".into(),
            color: [0.4, 0.4, 0.5, 0.8],
        });

        SceneDrawOutput {
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons: vec![],
            buttons,
            window_title: "Mahjuro".into(),
        }
    }
}
