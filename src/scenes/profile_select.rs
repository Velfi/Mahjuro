//! Profile selection screen — pick one of three profile slots.

use crate::persistence;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::{ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};
use super::start_screen::StartScreenScene;

const PROFILE_COUNT: usize = 3;

pub struct ProfileSelectScene {
    cursor: usize,
}

impl ProfileSelectScene {
    pub fn new(active_profile: usize) -> Self {
        Self {
            cursor: active_profile.min(PROFILE_COUNT - 1),
        }
    }

    /// Create with cursor pre-set to the currently active profile from settings.
    pub fn from_settings() -> Self {
        let settings = persistence::load_settings();
        Self::new(settings.active_profile)
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for a in ctx.actions {
            match a {
                UiAction::FocusNext | UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % PROFILE_COUNT;
                }
                UiAction::FocusPrev | UiAction::FocusUp => {
                    self.cursor = (self.cursor + PROFILE_COUNT - 1) % PROFILE_COUNT;
                }
                UiAction::Confirm | UiAction::CommitDiscard => {
                    *ctx.switch_profile = Some(self.cursor);
                    return Some(Scene::StartScreen(StartScreenScene::new()));
                }
                UiAction::Cancel | UiAction::Pause => {
                    return Some(Scene::StartScreen(StartScreenScene::new()));
                }
                // Mouse clicks on profile cards.
                UiAction::SortBySuit => {
                    self.cursor = 0;
                    *ctx.switch_profile = Some(0);
                    return Some(Scene::StartScreen(StartScreenScene::new()));
                }
                UiAction::SortByRank => {
                    self.cursor = 1;
                    *ctx.switch_profile = Some(1);
                    return Some(Scene::StartScreen(StartScreenScene::new()));
                }
                UiAction::ScoreHand => {
                    self.cursor = 2;
                    *ctx.switch_profile = Some(2);
                    return Some(Scene::StartScreen(StartScreenScene::new()));
                }
                _ => {}
            }
        }
        None
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.04, 0.05, 0.08, 1.0],
        }];
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();

        // Title.
        let title_h = (48.0 * scale).max(28.0);
        let title_y = h * 0.06;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "Select Profile".into(),
            color: [1.0, 0.95, 0.7, 1.0],
        });

        // Profile cards.
        let summaries = persistence::all_profile_summaries();
        let card_w = (260.0 * scale).min(w * 0.8);
        let card_h = (160.0 * scale).max(100.0);
        let card_gap = (16.0 * scale).max(8.0);
        let total_h = PROFILE_COUNT as f32 * card_h + (PROFILE_COUNT - 1) as f32 * card_gap;
        let start_y = title_y + title_h + (h - title_y - title_h - total_h) * 0.35;
        let card_x = (w - card_w) * 0.5;

        // Actions for each card's mouse click.
        let card_actions = [UiAction::SortBySuit, UiAction::SortByRank, UiAction::ScoreHand];

        for (i, summary) in summaries.iter().enumerate() {
            let card_y = start_y + i as f32 * (card_h + card_gap);
            let is_focused = i == self.cursor;
            let is_active = i == ctx.active_profile;

            // Card background.
            let bg_color = if is_focused {
                [0.18, 0.28, 0.42, 0.95]
            } else {
                [0.10, 0.12, 0.20, 0.85]
            };
            instances.push(GpuInstance {
                rect: [card_x, card_y, card_w, card_h],
                color: bg_color,
            });

            // Active indicator stripe on left edge.
            if is_active {
                let stripe_w = 4.0 * scale;
                instances.push(GpuInstance {
                    rect: [card_x, card_y, stripe_w, card_h],
                    color: [0.4, 0.8, 0.5, 1.0],
                });
            }

            // Selection highlight border (top and bottom lines).
            if is_focused {
                let border = 2.0 * scale;
                instances.push(GpuInstance {
                    rect: [card_x, card_y, card_w, border],
                    color: [0.5, 0.7, 1.0, 0.8],
                });
                instances.push(GpuInstance {
                    rect: [card_x, card_y + card_h - border, card_w, border],
                    color: [0.5, 0.7, 1.0, 0.8],
                });
            }

            let pad_x = 16.0 * scale;
            let pad_y = 10.0 * scale;
            let line_h = (22.0 * scale).max(14.0);
            let small_h = (17.0 * scale).max(11.0);

            // Profile header line.
            let header_text = if is_active {
                format!("Profile {}  (active)", i + 1)
            } else {
                format!("Profile {}", i + 1)
            };
            let header_rect = [card_x + pad_x, card_y + pad_y, card_w - pad_x * 2.0, line_h];
            text_labels.push(TextLabel {
                rect: header_rect,
                text: header_text,
                color: if is_focused {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    [0.8, 0.8, 0.9, 1.0]
                },
            });

            if summary.exists {
                let stat_x = card_x + pad_x;
                let stat_w = card_w - pad_x * 2.0;
                let line1_y = card_y + pad_y + line_h + pad_y * 0.5;
                let stat_color = [0.6, 0.65, 0.75, 1.0];

                // Level.
                text_labels.push(TextLabel {
                    rect: [stat_x, line1_y, stat_w, small_h],
                    text: format!("Level {}", summary.level),
                    color: stat_color,
                });

                // Runs completed.
                let line2_y = line1_y + small_h + pad_y * 0.3;
                text_labels.push(TextLabel {
                    rect: [stat_x, line2_y, stat_w, small_h],
                    text: format!(
                        "{} run{} completed",
                        summary.runs_completed,
                        if summary.runs_completed == 1 { "" } else { "s" }
                    ),
                    color: stat_color,
                });

                // High score.
                let line3_y = line2_y + small_h + pad_y * 0.3;
                text_labels.push(TextLabel {
                    rect: [stat_x, line3_y, stat_w, small_h],
                    text: format!("Best score: {}", summary.high_score),
                    color: stat_color,
                });

                // Saved game indicator.
                if summary.has_saved_run {
                    let line4_y = line3_y + small_h + pad_y * 0.3;
                    text_labels.push(TextLabel {
                        rect: [stat_x, line4_y, stat_w, small_h],
                        text: "Saved game in progress".into(),
                        color: [0.5, 0.9, 0.6, 1.0],
                    });
                }
            } else {
                // Empty profile.
                let empty_y = card_y + pad_y + line_h + pad_y;
                text_labels.push(TextLabel {
                    rect: [card_x + pad_x, empty_y, card_w - pad_x * 2.0, small_h],
                    text: "Empty — start a new adventure".into(),
                    color: [0.4, 0.4, 0.5, 0.7],
                });
            }

            buttons.push(ButtonDef {
                rect: (card_x, card_y, card_w, card_h),
                action: card_actions[i],
            });
        }

        // Hint text at bottom.
        let hint_h = (18.0 * scale).max(12.0);
        let hint_y = h - hint_h - (12.0 * scale);
        text_labels.push(TextLabel {
            rect: [0.0, hint_y, w, hint_h],
            text: "Up/Down to browse  |  Enter to select  |  Esc to go back".into(),
            color: [0.4, 0.4, 0.5, 0.8],
        });

        SceneDrawOutput {
            background: Default::default(),
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons: vec![],
            buttons,
            window_title: "Mahjuro — Select Profile".into(),
            departing_indices: vec![],
            hint_indices: vec![],
        }
    }
}
