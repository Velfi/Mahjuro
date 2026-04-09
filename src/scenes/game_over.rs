//! Game over scene — shown when the player exhausts plays without reaching the target.

use crate::game::run::RunState;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use crate::render::draw_cmd::UiFrame;

use super::start_screen::StartScreenScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DismissAction;

pub struct GameOverScene {
    pub final_score: u32,
    pub target_score: u32,
    pub won: bool,
    tree: TreeState,
}

impl GameOverScene {
    pub fn new(final_score: u32, target_score: u32) -> Self {
        Self {
            final_score,
            target_score,
            won: false,
            tree: TreeState::new(),
        }
    }

    /// Construct a victory screen shown after defeating the final-ante Boss.
    pub fn victory(final_score: u32, target_score: u32) -> Self {
        Self {
            final_score,
            target_score,
            won: true,
            tree: TreeState::new(),
        }
    }

    fn flat_items(&self, w: f32, h: f32) -> [FlatItem<DismissAction>; 1] {
        [FlatItem::new(FocusId(0), [0.0, 0.0, w, h], DismissAction)]
    }
}

impl SceneBehavior for GameOverScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let items = self.flat_items(ctx.layout.window_w, ctx.layout.window_h);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                ui_scale: ctx.ui_scale,
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if action.is_some() {
            *ctx.run = RunState::new_demo();
            return Some(Scene::StartScreen(StartScreenScene::new()));
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let headline = if self.won { "VICTORY" } else { "DEFEAT" };
        let headline_color = if self.won {
            color::CHAMPAGNE
        } else {
            color::RUBY
        };
        let subtitle = if self.won {
            "Final ante cleared".to_string()
        } else {
            format!("{} / {}", self.final_score, self.target_score)
        };

        let headline_rect = [w * 0.1, h * 0.32, w * 0.8, h * 0.18];
        let subtitle_rect = [w * 0.1, h * 0.50, w * 0.8, h * 0.10];
        let hint_rect = [w * 0.1, h * 0.62, w * 0.8, h * 0.06];

        let items = self.flat_items(w, h);
        let mut buttons = Vec::new();
        self.tree.register_flat_buttons(&items, &mut buttons);

        let mut frame = UiFrame::new();
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        });
        frame.text(TextLabel {
            rect: headline_rect,
            text: headline.to_string(),
            color: headline_color,
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: subtitle_rect,
            text: subtitle,
            color: color::PARCHMENT,
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: hint_rect,
            text: "Press Enter to continue".to_string(),
            color: color::MIST,
            ..Default::default()
        });

        frame.buttons = buttons;
        frame.window_title = if self.won {
            "Victory! — Final ante cleared — Press Enter to continue".to_string()
        } else {
            format!(
                "Game Over — {} / {} — Press Enter to restart",
                self.final_score, self.target_score
            )
        };
        frame
    }
}
