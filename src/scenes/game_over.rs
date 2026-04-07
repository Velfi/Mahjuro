//! Game over scene — shown when the player exhausts plays without reaching the target.

use crate::game::run::RunState;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::{ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};
use super::start_screen::StartScreenScene;

pub struct GameOverScene {
    pub final_score: u32,
    pub target_score: u32,
    pub won: bool,
}

impl GameOverScene {
    pub fn new(final_score: u32, target_score: u32) -> Self {
        Self { final_score, target_score, won: false }
    }

    /// Construct a victory screen shown after defeating the final-ante Boss.
    pub fn victory(final_score: u32, target_score: u32) -> Self {
        Self { final_score, target_score, won: true }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for a in ctx.actions {
            if matches!(a, UiAction::Confirm | UiAction::CommitDiscard) {
                *ctx.run = RunState::new_demo();
                return Some(Scene::StartScreen(StartScreenScene::new()));
            }
        }
        None
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let bg_color = if self.won {
            [0.05, 0.10, 0.18, 1.0]
        } else {
            [0.10, 0.06, 0.10, 1.0]
        };
        let headline = if self.won { "VICTORY" } else { "DEFEAT" };
        let headline_color = if self.won {
            [1.0, 0.85, 0.35, 1.0]
        } else {
            [0.95, 0.55, 0.55, 1.0]
        };
        let subtitle = if self.won {
            "Final ante cleared".to_string()
        } else {
            format!("{} / {}", self.final_score, self.target_score)
        };

        // Three stacked labels: headline, score line, hint.
        let headline_rect = [w * 0.1, h * 0.32, w * 0.8, h * 0.18];
        let subtitle_rect = [w * 0.1, h * 0.50, w * 0.8, h * 0.10];
        let hint_rect = [w * 0.1, h * 0.62, w * 0.8, h * 0.06];

        // Whole-screen click target — there's only one action, so any click
        // dismisses. Routes through Confirm in update().
        let dismiss_button = ButtonDef::ui((0.0, 0.0, w, h), UiAction::Confirm);

        let title = if self.won {
            "Victory! — Final ante cleared — Press Enter to continue".to_string()
        } else {
            format!(
                "Game Over — {} / {} — Press Enter to restart",
                self.final_score, self.target_score
            )
        };

        SceneDrawOutput {
            background: Default::default(),
            instances: vec![GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: bg_color,
            }],
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels: vec![
                TextLabel {
                    rect: headline_rect,
                    text: headline.to_string(),
                    color: headline_color,
                },
                TextLabel {
                    rect: subtitle_rect,
                    text: subtitle,
                    color: [0.95, 0.95, 0.95, 1.0],
                },
                TextLabel {
                    rect: hint_rect,
                    text: "Press Enter to continue".to_string(),
                    color: [0.75, 0.75, 0.80, 1.0],
                },
            ],
            relic_icons: vec![],
            buttons: vec![dismiss_button],
            window_title: title,
            departing_indices: vec![],
            hint_indices: vec![],
        }
    }
}
