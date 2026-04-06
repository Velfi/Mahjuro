//! Game over scene — shown when the player exhausts plays without reaching the target.

use crate::game::run::RunState;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::{DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};
use super::start_screen::StartScreenScene;

pub struct GameOverScene {
    pub final_score: u32,
    pub target_score: u32,
}

impl GameOverScene {
    pub fn new(final_score: u32, target_score: u32) -> Self {
        Self { final_score, target_score }
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
        SceneDrawOutput {
            instances: vec![GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: [0.18, 0.04, 0.04, 1.0],
            }],
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels: vec![TextLabel {
                rect: [w * 0.1, h * 0.3, w * 0.8, h * 0.4],
                text: format!(
                    "Game Over   {} / {}   Press Enter to restart",
                    self.final_score, self.target_score
                ),
                color: [1.0, 1.0, 1.0, 1.0],
            }],
            relic_icons: vec![],
            buttons: vec![],
            window_title: format!(
                "Game Over — {} / {} — Press Enter to restart",
                self.final_score, self.target_score
            ),
        }
    }
}
