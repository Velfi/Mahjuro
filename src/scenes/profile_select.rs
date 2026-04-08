//! Profile selection screen — pick one of three profile slots.

use crate::persistence;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::start_screen::StartScreenScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneDrawOutput, SceneTransition, UpdateCtx};

const PROFILE_COUNT: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PickProfile(usize);

pub struct ProfileSelectScene {
    tree: TreeState,
}

impl ProfileSelectScene {
    pub fn new(active_profile: usize) -> Self {
        let mut tree = TreeState::new();
        tree.set_focus(FocusId(active_profile.min(PROFILE_COUNT - 1) as u32));
        Self { tree }
    }

    /// Create with cursor pre-set to the currently active profile from settings.
    pub fn from_settings() -> Self {
        let settings = persistence::load_settings();
        Self::new(settings.active_profile)
    }

    fn cursor(&self) -> usize {
        self.tree
            .focused()
            .map(|f| f.0 as usize)
            .unwrap_or(0)
            .min(PROFILE_COUNT - 1)
    }

    /// Single source of truth for profile card layout — used by both
    /// `update()` (hit-test) and `draw()` (rendering + button registration).
    fn card_rects(window_w: f32, window_h: f32) -> Vec<[f32; 4]> {
        let w = window_w;
        let h = window_h;
        let scale = (w.min(h)) / 600.0;
        let title_h = (48.0 * scale).max(28.0);
        let title_y = h * 0.06;
        let card_w = (260.0 * scale).min(w * 0.8);
        let card_h = (160.0 * scale).max(100.0);
        let card_gap = (16.0 * scale).max(8.0);
        let total_h = PROFILE_COUNT as f32 * card_h + (PROFILE_COUNT - 1) as f32 * card_gap;
        let start_y = title_y + title_h + (h - title_y - title_h - total_h) * 0.35;
        let card_x = (w - card_w) * 0.5;
        (0..PROFILE_COUNT)
            .map(|i| {
                let card_y = start_y + i as f32 * (card_h + card_gap);
                [card_x, card_y, card_w, card_h]
            })
            .collect()
    }

    fn flat_items(window_w: f32, window_h: f32) -> Vec<FlatItem<PickProfile>> {
        Self::card_rects(window_w, window_h)
            .into_iter()
            .enumerate()
            .map(|(i, rect)| FlatItem::new(FocusId(i as u32), rect, PickProfile(i)))
            .collect()
    }
}

impl SceneBehavior for ProfileSelectScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let items = Self::flat_items(ctx.layout.window_w, ctx.layout.window_h);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
            },
        );

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause) {
                return Some(Scene::StartScreen(StartScreenScene::new()));
            }
        }

        if let Some(PickProfile(idx)) = action {
            *ctx.switch_profile = Some(idx);
            return Some(Scene::StartScreen(StartScreenScene::new()));
        }
        None
    }

    fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        }];
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();

        // Title.
        let title_h = (48.0 * scale).max(28.0);
        let title_y = h * 0.06;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "Select Profile".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // Profile cards — single source of truth via card_rects().
        let summaries = persistence::all_profile_summaries();
        let card_rects = Self::card_rects(w, h);
        let cursor = self.cursor();

        for (i, summary) in summaries.iter().enumerate() {
            let [card_x, card_y, card_w, card_h] = card_rects[i];
            let is_focused = i == cursor;
            let is_active = i == ctx.active_profile;

            // Card background.
            let bg_color = if is_focused {
                color::DUSK
            } else {
                color::INDIGO
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
                    color: color::JADE,
                });
            }

            // Selection highlight border (top and bottom gold lines).
            if is_focused {
                let border = 2.0 * scale;
                instances.push(GpuInstance {
                    rect: [card_x, card_y, card_w, border],
                    color: color::GOLD,
                });
                instances.push(GpuInstance {
                    rect: [card_x, card_y + card_h - border, card_w, border],
                    color: color::GOLD,
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
                    color::CHAMPAGNE
                } else {
                    color::PARCHMENT
                },
                ..Default::default()
            });

            if summary.exists {
                let stat_x = card_x + pad_x;
                let stat_w = card_w - pad_x * 2.0;
                let line1_y = card_y + pad_y + line_h + pad_y * 0.5;
                let stat_color = color::MIST;

                // Level.
                text_labels.push(TextLabel {
                    rect: [stat_x, line1_y, stat_w, small_h],
                    text: format!("Level {}", summary.level),
                    color: stat_color,
                    ..Default::default()
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
                    ..Default::default()
                });

                // High score.
                let line3_y = line2_y + small_h + pad_y * 0.3;
                text_labels.push(TextLabel {
                    rect: [stat_x, line3_y, stat_w, small_h],
                    text: format!("Best score: {}", summary.high_score),
                    color: stat_color,
                    ..Default::default()
                });

                // Saved game indicator.
                if summary.has_saved_run {
                    let line4_y = line3_y + small_h + pad_y * 0.3;
                    text_labels.push(TextLabel {
                        rect: [stat_x, line4_y, stat_w, small_h],
                        text: "Saved game in progress".into(),
                        color: color::JADE,
                        ..Default::default()
                    });
                }
            } else {
                // Empty profile.
                let empty_y = card_y + pad_y + line_h + pad_y;
                text_labels.push(TextLabel {
                    rect: [card_x + pad_x, empty_y, card_w - pad_x * 2.0, small_h],
                    text: "Empty — start a new adventure".into(),
                    color: color::SLATE,
                    ..Default::default()
                });
            }
        }

        // Single hit-target list shared with update() — no layout drift.
        let items = Self::flat_items(w, h);
        self.tree.register_flat_buttons(&items, &mut buttons);

        // Hint text at bottom.
        let hint_h = (18.0 * scale).max(12.0);
        let hint_y = h - hint_h - (12.0 * scale);
        text_labels.push(TextLabel {
            rect: [0.0, hint_y, w, hint_h],
            text: "Up/Down to browse  |  Enter to select  |  Esc to go back".into(),
            color: color::SLATE,
            ..Default::default()
        });

        SceneDrawOutput {
            background: Default::default(),
            tray_instances: vec![],
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
            flame_instances: vec![],
            point_lights: vec![],
            candles: vec![],
            relic_placements: vec![],
            draw_table: false,
            wind_gusts: Vec::new(),
        }
    }
}
