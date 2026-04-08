//! Game over scene — shown when the player exhausts plays without reaching the target.

use crate::game::run::RunState;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::start_screen::StartScreenScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneDrawOutput, SceneTransition, UpdateCtx};

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
            },
        );
        if action.is_some() {
            *ctx.run = RunState::new_demo();
            return Some(Scene::StartScreen(StartScreenScene::new()));
        }
        None
    }

    fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let bg_color = color::OBSIDIAN;
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

        // Three stacked labels: headline, score line, hint.
        let headline_rect = [w * 0.1, h * 0.32, w * 0.8, h * 0.18];
        let subtitle_rect = [w * 0.1, h * 0.50, w * 0.8, h * 0.10];
        let hint_rect = [w * 0.1, h * 0.62, w * 0.8, h * 0.06];

        // Whole-screen click target — registered via the tree so click ids
        // route back through update().
        let items = self.flat_items(w, h);
        let mut buttons = Vec::new();
        self.tree.register_flat_buttons(&items, &mut buttons);

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
            tray_instances: vec![],
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
                    ..Default::default()
                },
                TextLabel {
                    rect: subtitle_rect,
                    text: subtitle,
                    color: color::PARCHMENT,
                    ..Default::default()
                },
                TextLabel {
                    rect: hint_rect,
                    text: "Press Enter to continue".to_string(),
                    color: color::MIST,
                    ..Default::default()
                },
            ],
            relic_icons: vec![],
            buttons,
            window_title: title,
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
