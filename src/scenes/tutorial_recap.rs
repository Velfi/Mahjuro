//! Post-lesson recap scene shown between tutorial rounds.
//!
//! Summarises what the player just learned and previews the next lesson
//! before transitioning to gameplay (or shop).

use crate::game::tutorial::{LESSON_COUNT, lesson_def};
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::gameplay::GameplayScene;
use super::shop::ShopScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Continue;

pub struct TutorialRecapScene {
    /// The lesson that was just completed (1-indexed).
    completed_lesson: u32,
    /// Whether the shop follows this recap (lesson 8+).
    shop_follows: bool,
    tree: TreeState,
}

impl TutorialRecapScene {
    pub fn new(completed_lesson: u32, shop_follows: bool) -> Self {
        Self {
            completed_lesson,
            shop_follows,
            tree: TreeState::new(),
        }
    }

    fn flat_items(&self, w: f32, h: f32) -> [FlatItem<Continue>; 1] {
        // Full-screen hit target — any click/key continues.
        [FlatItem::new(FocusId(0), [0.0, 0.0, w, h], Continue)]
    }
}

impl SceneBehavior for TutorialRecapScene {
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
            return Some(if self.shop_follows {
                Scene::Shop(ShopScene::new(ctx.run.run_number, ctx.run))
            } else {
                Scene::Gameplay(GameplayScene::new())
            });
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w / 800.0).min(h / 600.0).max(0.5) * ctx.ui_scale;

        let lesson = lesson_def(self.completed_lesson);

        let mut frame = UiFrame::new();

        // ── Background + celestial vignette ─────────────────────
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.0, 0.0, 0.0, 1.0],
        });
        frame.starfield();
        frame.golden_dust();

        // ── Central card ─────────────────────────────────────────
        let card_w = (500.0 * scale).min(w * 0.85);
        let card_h = (360.0 * scale).min(h * 0.7);
        let cx = (w - card_w) * 0.5;
        let cy = (h - card_h) * 0.5 - 10.0 * scale;
        frame.quad(GpuInstance {
            rect: [cx, cy, card_w, card_h],
            color: color::MIDNIGHT,
        });

        // Thin gold border around card.
        let bw = (2.0 * scale).max(1.0);
        // Top
        frame.quad(GpuInstance {
            rect: [cx, cy, card_w, bw],
            color: color::BRASS,
        });
        // Bottom
        frame.quad(GpuInstance {
            rect: [cx, cy + card_h - bw, card_w, bw],
            color: color::BRASS,
        });
        // Left
        frame.quad(GpuInstance {
            rect: [cx, cy, bw, card_h],
            color: color::BRASS,
        });
        // Right
        frame.quad(GpuInstance {
            rect: [cx + card_w - bw, cy, bw, card_h],
            color: color::BRASS,
        });

        // ── Lesson number ────────────────────────────────────────
        let lesson_num_h = 24.0 * scale;
        frame.text(TextLabel {
            rect: [cx, cy + 16.0 * scale, card_w, lesson_num_h],
            text: format!("Lesson {} / {}", self.completed_lesson, LESSON_COUNT),
            color: color::SLATE,
            font_px: Some(lesson_num_h * 0.7),
            align: TextAlign::Center,
            ..Default::default()
        });

        // ── Headline (first recap line) ──────────────────────────
        let headline = lesson.recap.first().copied().unwrap_or("Lesson Complete");
        let headline_h = 38.0 * scale;
        let headline_y = cy + 44.0 * scale;
        frame.text(TextLabel {
            rect: [
                cx + 20.0 * scale,
                headline_y,
                card_w - 40.0 * scale,
                headline_h,
            ],
            text: headline.to_string(),
            color: color::CHAMPAGNE,
            font_px: Some(headline_h * 0.75),
            align: TextAlign::Center,
            ..Default::default()
        });

        // ── Divider ──────────────────────────────────────────────
        let div_y = headline_y + headline_h + 8.0 * scale;
        let div_inset = card_w * 0.15;
        frame.quad(GpuInstance {
            rect: [cx + div_inset, div_y, card_w - div_inset * 2.0, bw],
            color: color::alpha(color::BRASS, 0.5),
        });

        // ── Bullet points (remaining recap lines) ────────────────
        let bullet_y_start = div_y + 16.0 * scale;
        let bullet_font = 16.8 * scale;
        let bullet_line_h = bullet_font * 1.3;
        let bullet_pad = 6.0 * scale;
        let bullet_w = card_w - 60.0 * scale;
        let bullet_cpl = ((bullet_w / (bullet_font * 0.5)) as usize).max(20);
        let mut by = bullet_y_start;
        for &line in lesson.recap.iter().skip(1) {
            let wrapped = simple_word_wrap(&format!("\u{2022}  {line}"), bullet_cpl);
            let line_count = wrapped.matches('\n').count() + 1;
            let h = bullet_line_h * line_count as f32;
            frame.text(TextLabel {
                rect: [cx + 30.0 * scale, by, bullet_w, h],
                text: wrapped,
                color: color::PARCHMENT,
                font_px: Some(bullet_font),
                align: TextAlign::Left,
                ..Default::default()
            });
            by += h + bullet_pad;
        }

        // ── "Up next" preview ────────────────────────────────────
        if (self.completed_lesson as usize) < LESSON_COUNT {
            let next = lesson_def(self.completed_lesson + 1);
            let preview_y = cy + card_h - 110.0 * scale;

            frame.text(TextLabel {
                rect: [
                    cx + 20.0 * scale,
                    preview_y,
                    card_w - 40.0 * scale,
                    22.0 * scale,
                ],
                text: "Up Next".to_string(),
                color: color::GOLD,
                font_px: Some(18.0 * scale),
                align: TextAlign::Center,
                ..Default::default()
            });

            let intro_font = 15.0 * scale;
            let intro_w = card_w - 60.0 * scale;
            // Rough chars-per-line: rect width / (font_px * 0.5).
            let chars_per_line = ((intro_w / (intro_font * 0.5)) as usize).max(20);
            let wrapped = simple_word_wrap(next.intro_text, chars_per_line);
            let line_count = wrapped.matches('\n').count() + 1;
            let intro_h = intro_font * 1.3 * line_count as f32;

            frame.text(TextLabel {
                rect: [
                    cx + 30.0 * scale,
                    preview_y + 26.0 * scale,
                    intro_w,
                    intro_h,
                ],
                text: wrapped,
                color: color::MIST,
                font_px: Some(intro_font),
                align: TextAlign::Center,
                ..Default::default()
            });
        }

        // ── Flavor text (italic feel) ────────────────────────────
        let flavor_y = cy + card_h - 34.0 * scale;
        frame.text(TextLabel {
            rect: [cx, flavor_y, card_w, 20.0 * scale],
            text: format!("\u{201c}{}\u{201d}", lesson.flavor_text),
            color: color::alpha(color::MIST, 0.6),
            font_px: Some(13.0 * scale),
            align: TextAlign::Center,
            ..Default::default()
        });

        // ── Continue hint ────────────────────────────────────────
        let hint_y = cy + card_h + 18.0 * scale;
        frame.text(TextLabel {
            rect: [0.0, hint_y, w, 22.0 * scale],
            text: "Press Enter to continue".to_string(),
            color: color::MIST,
            font_px: Some(16.0 * scale),
            align: TextAlign::Center,
            ..Default::default()
        });

        // Register full-screen continue button.
        let items = self.flat_items(w, h);
        let mut buttons = Vec::new();
        self.tree.register_flat_buttons(&items, &mut buttons);
        frame.buttons = buttons;

        frame.window_title = format!(
            "Lesson {} Complete \u{2014} {}",
            self.completed_lesson, headline
        );
        frame
    }
}

/// Greedy word-wrap by character budget, returning lines joined with `\n`.
fn simple_word_wrap(text: &str, max_chars: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines.join("\n")
}
