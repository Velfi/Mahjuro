//! Start screen — title screen shown when the game launches.

use crate::game::run::RunState;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::{DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};
use super::shop::ShopScene;

pub struct StartScreenScene;

impl StartScreenScene {
    pub fn new() -> Self {
        Self
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for a in ctx.actions {
            if matches!(a, UiAction::Confirm | UiAction::CommitDiscard) {
                *ctx.run = RunState::new_demo();
                return Some(Scene::Shop(ShopScene::new(ctx.run.run_number, &ctx.run.relics)));
            }
        }
        None
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let sp = ctx.layout.score_panel;
        let ms = ctx.layout.modifier_strip;
        SceneDrawOutput {
            instances: vec![GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: [0.04, 0.05, 0.08, 1.0],
            }],
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels: vec![
                TextLabel {
                    rect: [sp.x, sp.y, sp.w, sp.h],
                    text: "MAHJURO".into(),
                    color: [1.0, 1.0, 1.0, 1.0],
                },
                TextLabel {
                    rect: [ms.x, ms.y, ms.w, ms.h],
                    text: "Press Enter to begin".into(),
                    color: [1.0, 1.0, 1.0, 1.0],
                },
            ],
            relic_icons: vec![],
            buttons: vec![],
            window_title: "Mahjuro — Press Enter to begin".into(),
        }
    }
}
