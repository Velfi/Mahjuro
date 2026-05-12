//! Tile literacy scene — brief visual introduction to mahjong tile types.
//!
//! Shown once at the very start of the tutorial (before Lesson 1) to teach
//! new players how to read tiles: number suits, honor tiles, and matching.

use crate::audio::SfxId;
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::gameplay::GameplayScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Continue;

pub struct TileLiteracyScene {
    tree: TreeState,
}

impl TileLiteracyScene {
    fn flat_items(&self, w: f32, h: f32) -> [FlatItem<Continue>; 1] {
        [FlatItem::new(FocusId(0), [0.0, 0.0, w, h], Continue)]
    }
}

impl SceneBehavior for TileLiteracyScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let items = self.flat_items(ctx.layout.window_w, ctx.layout.window_h);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        if action.is_some() {
            ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
            return Some(Scene::Gameplay(GameplayScene::new()));
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w / 800.0).min(h / 600.0).max(0.5);

        let mut frame = UiFrame::new();

        // ── Background ──────────────────────────────────────────
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.0, 0.0, 0.0, 1.0],
        });
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        if ctx.effect_layers.golden_dust {
            frame.golden_dust();
        }

        // ── Central card ────────────────────────────────────────
        let card_w = (540.0 * scale).min(w * 0.88);
        let card_h = (420.0 * scale).min(h * 0.78);
        let cx = (w - card_w) * 0.5;
        let cy = (h - card_h) * 0.5 - 10.0 * scale;
        frame.quad(GpuInstance {
            rect: [cx, cy, card_w, card_h],
            color: color::WALNUT_DEEP,
        });

        // Gold border.
        let bw = (2.0 * scale).max(1.0);
        frame.quad(GpuInstance {
            rect: [cx, cy, card_w, bw],
            color: color::BRASS,
        });
        frame.quad(GpuInstance {
            rect: [cx, cy + card_h - bw, card_w, bw],
            color: color::BRASS,
        });
        frame.quad(GpuInstance {
            rect: [cx, cy, bw, card_h],
            color: color::BRASS,
        });
        frame.quad(GpuInstance {
            rect: [cx + card_w - bw, cy, bw, card_h],
            color: color::BRASS,
        });

        // ── Title ───────────────────────────────────────────────
        let title_h = 34.0 * scale;
        let title_y = cy + 18.0 * scale;
        frame.text(TextLabel {
            rect: [cx, title_y, card_w, title_h],
            text: "Know Your Tiles".to_string(),
            color: color::CHAMPAGNE,
            font_px: Some(title_h * 0.8),
            align: TextAlign::Center,
            ..Default::default()
        });

        // ── Divider ─────────────────────────────────────────────
        let div_y = title_y + title_h + 6.0 * scale;
        let div_inset = card_w * 0.12;
        frame.quad(GpuInstance {
            rect: [cx + div_inset, div_y, card_w - div_inset * 2.0, bw],
            color: color::alpha(color::BRASS, 0.5),
        });

        // ── Content lines ───────────────────────────────────────
        let content_font = 16.0 * scale;
        let line_h = content_font * 1.5;
        let content_x = cx + 28.0 * scale;
        let content_w = card_w - 56.0 * scale;
        let chars_per_line = ((content_w / (content_font * 0.5)) as usize).max(20);
        let mut y = div_y + 16.0 * scale;

        let lines: &[(&str, [f32; 4])] = &[
            (
                "Mahjong uses tiles instead of cards. Here\u{2019}s what you\u{2019}ll see:",
                color::PARCHMENT,
            ),
            ("", color::PARCHMENT), // spacer
            (
                "\u{2022}  Number Suits \u{2014} Bamboo, Coins, and Characters, ranked 1\u{2013}9. Tiles of the same suit and rank are identical.",
                color::PARCHMENT,
            ),
            (
                "\u{2022}  Honor Tiles \u{2014} Winds (East, South, West, North) and Dragons (Green, Red, White). No rank \u{2014} they match by name.",
                color::PARCHMENT,
            ),
            ("", color::PARCHMENT), // spacer
            (
                "\u{2022}  Matching: Two tiles match if they share the same suit AND rank. A 3 of Bamboo matches another 3 of Bamboo, but not a 3 of Coins.",
                color::PARCHMENT,
            ),
            (
                "\u{2022}  Sequences: Three consecutive tiles in the same number suit (like 2\u{2013}3\u{2013}4 of Coins). Honor tiles can\u{2019}t form sequences.",
                color::PARCHMENT,
            ),
            ("", color::PARCHMENT), // spacer
            (
                "Don\u{2019}t worry about memorizing everything \u{2014} the game will guide you!",
                color::alpha(color::GOLD, 0.9),
            ),
        ];

        for &(text, text_color) in lines {
            if text.is_empty() {
                y += line_h * 0.4;
                continue;
            }
            let wrapped = simple_word_wrap(text, chars_per_line);
            let line_count = wrapped.matches('\n').count() + 1;
            let block_h = line_h * line_count as f32;
            frame.text(TextLabel {
                rect: [content_x, y, content_w, block_h],
                text: wrapped,
                color: text_color,
                font_px: Some(content_font),
                align: TextAlign::Left,
                ..Default::default()
            });
            y += block_h + 4.0 * scale;
        }

        // ── Continue hint ───────────────────────────────────────
        let hint_y = cy + card_h + 18.0 * scale;
        frame.text(TextLabel {
            rect: [0.0, hint_y, w, 22.0 * scale],
            text: "Press Enter to start".to_string(),
            color: color::STONE,
            font_px: Some(16.0 * scale),
            align: TextAlign::Center,
            ..Default::default()
        });

        // Register full-screen continue button.
        let items = self.flat_items(w, h);
        let mut buttons = Vec::new();
        self.tree.register_flat_buttons(&items, &mut buttons);
        frame.buttons = buttons;

        frame.window_title = "Know Your Tiles".to_string();
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
