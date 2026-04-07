//! Start screen — title screen with main menu.

use crate::game::run::RunState;
use crate::persistence;
use crate::render::theme::{ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget;

use super::collection::CollectionScene;
use super::gameplay::GameplayScene;
use super::options::OptionsScene;
use super::profile_select::ProfileSelectScene;
use super::shop::ShopScene;
use super::solitaire::SolitaireScene;
use super::{DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};

/// Menu items when a game is in progress.
const IP_CONTINUE: usize = 0;
const IP_NEW_GAME: usize = 1;
const IP_SOLITAIRE: usize = 2;
const IP_PROFILE: usize = 3;
const IP_COLLECTION: usize = 4;
const IP_OPTIONS: usize = 5;
const IP_QUIT: usize = 6;
const IP_COUNT: usize = 7;

/// Menu items when no game is in progress.
const NP_PLAY: usize = 0;
const NP_SOLITAIRE: usize = 1;
const NP_PROFILE: usize = 2;
const NP_COLLECTION: usize = 3;
const NP_OPTIONS: usize = 4;
const NP_QUIT: usize = 5;
const NP_COUNT: usize = 6;

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

        // Mouse hover → focus the menu item under the pointer so the gold
        // highlight follows the cursor. Layout must match draw().
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0;
        let title_h = (64.0 * scale).max(32.0);
        let title_y = h * 0.08;
        let prof_y = title_y + title_h + h * 0.04;
        let prof_h = (24.0 * scale).max(16.0);
        let btn_w = (200.0 * scale).min(w * 0.5);
        let btn_h = (38.0 * scale).max(24.0);
        let btn_gap = (12.0 * scale).max(6.0);
        let menu_start_y = prof_y + prof_h + h * 0.06;
        let btn_x = (w - btn_w) * 0.5;
        let (cx, cy) = ctx.cursor_pos;
        for i in 0..count {
            let by = menu_start_y + i as f32 * (btn_h + btn_gap);
            if cx >= btn_x && cx <= btn_x + btn_w && cy >= by && cy <= by + btn_h {
                self.cursor = i;
                break;
            }
        }

        for a in ctx.actions {
            match a {
                UiAction::FocusDown | UiAction::FocusNext => {
                    self.cursor = (self.cursor + 1) % count;
                }
                UiAction::FocusUp | UiAction::FocusPrev => {
                    self.cursor = (self.cursor + count - 1) % count;
                }
                // Confirm — mouse clicks land here too because hover-focus
                // above already pointed self.cursor at the hovered button.
                UiAction::Confirm | UiAction::CommitDiscard => {
                    return self.activate(self.cursor, in_progress, ctx.run, ctx.quit_requested);
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
                IP_SOLITAIRE => return Some(Scene::Solitaire(SolitaireScene::new())),
                IP_PROFILE => {
                    return Some(Scene::ProfileSelect(ProfileSelectScene::from_settings()));
                }
                IP_COLLECTION => return Some(Scene::Collection(CollectionScene::new())),
                IP_OPTIONS => return Some(Scene::Options(OptionsScene::new())),
                IP_QUIT => *quit_requested = true,
                _ => {}
            }
        } else {
            match idx {
                NP_PLAY => return self.start_game(run),
                NP_SOLITAIRE => return Some(Scene::Solitaire(SolitaireScene::new())),
                NP_PROFILE => {
                    return Some(Scene::ProfileSelect(ProfileSelectScene::from_settings()));
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
            color: color::OBSIDIAN,
        }];
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();

        // Title — gold serif display.
        let title_h = typography::size(typography::DISPLAY, h);
        let title_y = h * 0.08;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "M A H J U R O".into(),
            color: color::CHAMPAGNE,
        });

        // Active profile summary below title.
        let prof_y = title_y + title_h + h * 0.04;
        let prof_h = typography::size(typography::CAPTION, h);
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
            color: color::MIST,
        });

        // Build menu items depending on whether a game is in progress.
        let menu_labels: Vec<&str> = if in_progress {
            vec![
                "Continue",
                "New Game",
                "Solitaire",
                "Profile",
                "Collection",
                "Options",
                "Quit",
            ]
        } else {
            vec![
                "Play",
                "Solitaire",
                "Profile",
                "Collection",
                "Options",
                "Quit",
            ]
        };
        let count = menu_labels.len();

        let quit_idx = count - 1;
        let continue_idx = 0;

        let btn_w = (220.0 * scale).min(w * 0.55);
        let btn_h = (44.0 * scale).max(28.0);
        let btn_gap = (12.0 * scale).max(6.0);
        let menu_start_y = prof_y + prof_h + h * 0.06;
        let btn_x = (w - btn_w) * 0.5;

        for (i, label) in menu_labels.iter().enumerate() {
            let btn_y = menu_start_y + i as f32 * (btn_h + btn_gap);
            let is_focused = i == self.cursor;

            let variant = if i == continue_idx {
                ButtonVariant::Primary
            } else if i == quit_idx {
                ButtonVariant::Danger
            } else {
                ButtonVariant::Default
            };
            let state = if is_focused {
                ButtonState::Hover
            } else {
                ButtonState::Rest
            };

            // Click → Confirm; hover-focus in update() ensures self.cursor
            // already points at this button by the time the action is read.
            widget::push_button(
                &mut instances,
                &mut text_labels,
                &mut buttons,
                [btn_x, btn_y, btn_w, btn_h],
                label,
                variant,
                state,
                UiAction::Confirm,
            );
        }

        // Hint text at bottom.
        let hint_h = typography::size(typography::MICRO, h);
        let hint_y = h - hint_h - (12.0 * scale);
        text_labels.push(TextLabel {
            rect: [0.0, hint_y, w, hint_h],
            text: "Arrow keys to navigate  |  Enter/Space to select".into(),
            color: color::SLATE,
        });

        SceneDrawOutput {
            background: super::BackgroundId::Menu,
            tray_instances: vec![],
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons: vec![],
            buttons,
            window_title: "Mahjuro".into(),
            departing_indices: vec![],
            hint_indices: vec![],
            flame_instances: vec![],
            point_lights: vec![],
            candles: vec![],
            draw_table: false,
        }
    }
}
