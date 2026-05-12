//! End-of-tutorial summary shown after the onboarding finale.

use crate::audio::SfxId;
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::persistence;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::main_menu_exterior::MainMenuExteriorScene;
use super::meld_guide::MeldGuideScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SummaryAction {
    Continue,
    MeldGuide,
}

impl SummaryAction {
    fn id(self) -> FocusId {
        FocusId(0x8F00 + self as u32)
    }
}

pub struct TutorialSummaryScene {
    won: bool,
    tree: TreeState,
}

impl TutorialSummaryScene {
    pub fn new(won: bool) -> Self {
        Self {
            won,
            tree: TreeState::new(),
        }
    }

    fn flat_items(&self, w: f32, h: f32) -> Vec<FlatItem<SummaryAction>> {
        let scale = (w / 800.0).min(h / 600.0).max(0.5);
        let btn_w = (200.0 * scale).min(w * 0.38).max(140.0);
        let btn_h = (44.0 * scale).max(32.0);
        let gap = 16.0 * scale;
        let y = h - btn_h - 28.0 * scale;
        let pair_w = btn_w * 2.0 + gap;
        let start_x = (w - pair_w) * 0.5;
        vec![
            FlatItem::new(
                SummaryAction::MeldGuide.id(),
                [start_x, y, btn_w, btn_h],
                SummaryAction::MeldGuide,
            ),
            FlatItem::new(
                SummaryAction::Continue.id(),
                [start_x + btn_w + gap, y, btn_w, btn_h],
                SummaryAction::Continue,
            ),
        ]
    }
}

impl SceneBehavior for TutorialSummaryScene {
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
        match action {
            Some(SummaryAction::MeldGuide) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                *ctx.overlay_request = Some(super::OverlayRequest::Push(Box::new(
                    Scene::MeldGuide(MeldGuideScene::new()),
                )));
                None
            }
            Some(SummaryAction::Continue) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                let settings = persistence::load_settings();
                GameEngine::reset_to_demo(ctx.run, ctx.progress, &settings);
                Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()))
            }
            None => None,
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w / 800.0).min(h / 600.0).max(0.5);
        let mut texts = Vec::new();

        let title = if self.won {
            "Tutorial Complete"
        } else {
            "Tutorial Recap"
        };
        let subtitle = if self.won {
            "You beat The Iconoclast and finished onboarding."
        } else {
            "You reached the finale. Honors scored low under The Iconoclast — bamboo, circles, characters, and shop picks carried most runs."
        };
        let bullets = [
            "Pairs, triplets, and sequences become melds when you bank them into your structure.",
            "Full Hand and Chiitoitsu (seven pairs) are the two tutorial patterns to learn first.",
            "Score is chips × mult; yaku are the usual way to grow mult early.",
            "Relics passively help all shrines; ribbons level yaku; talismans stamp tiles; packs reshape the wall.",
            "Any time: open Pause → Meld Guide for pictures of every meld and yaku.",
        ];

        let mut frame = UiFrame::new();
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::WALNUT_INK,
        });
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        if self.won && ctx.effect_layers.golden_dust {
            frame.golden_dust();
        }

        let card_w = (620.0 * scale).min(w * 0.88);
        let card_h = (430.0 * scale).min(h * 0.80);
        let card_x = (w - card_w) * 0.5;
        let card_y = (h - card_h) * 0.5;
        let border = (2.0 * scale).max(1.0);

        frame.quad(GpuInstance {
            rect: [card_x, card_y, card_w, card_h],
            color: color::WALNUT_DEEP,
        });
        frame.quad(GpuInstance {
            rect: [card_x, card_y, card_w, border],
            color: color::BRASS,
        });
        frame.quad(GpuInstance {
            rect: [card_x, card_y + card_h - border, card_w, border],
            color: color::BRASS,
        });
        frame.quad(GpuInstance {
            rect: [card_x, card_y, border, card_h],
            color: color::BRASS,
        });
        frame.quad(GpuInstance {
            rect: [card_x + card_w - border, card_y, border, card_h],
            color: color::BRASS,
        });

        texts.push(TextLabel {
            rect: [
                card_x + 20.0 * scale,
                card_y + 22.0 * scale,
                card_w - 40.0 * scale,
                42.0 * scale,
            ],
            text: title.to_string(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(28.0 * scale),
            ..Default::default()
        });

        let subtitle_x = card_x + 34.0 * scale;
        let subtitle_y = card_y + 78.0 * scale;
        let subtitle_w = card_w - 68.0 * scale;
        let subtitle_font = typography::size(typography::BODY, h).max(16.0);
        let subtitle_lines = widget::wrap_text(subtitle, subtitle_w, subtitle_font);
        let subtitle_h = subtitle_lines.len().max(1) as f32 * subtitle_font * 1.3;
        widget::push_text_block(
            &mut texts,
            [subtitle_x, subtitle_y, subtitle_w, subtitle_h],
            subtitle,
            TextStyle {
                tier: typography::BODY,
                color: color::PARCHMENT,
                padding: 0.0,
                align: TextAlign::Center,
            },
            h,
        );

        let mut bullet_y = subtitle_y + subtitle_h + 20.0 * scale;
        for bullet in bullets {
            let bullet_text = format!("• {bullet}");
            let bullet_w = card_w - 72.0 * scale;
            let bullet_font = typography::size(typography::BODY, h).max(15.0);
            let bullet_lines = widget::wrap_text(&bullet_text, bullet_w, bullet_font);
            let bullet_h = bullet_lines.len().max(1) as f32 * bullet_font * 1.25;
            widget::push_text_block(
                &mut texts,
                [card_x + 36.0 * scale, bullet_y, bullet_w, bullet_h],
                &bullet_text,
                TextStyle {
                    tier: typography::BODY,
                    color: color::STONE,
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
            );
            bullet_y += bullet_h + 10.0 * scale;
        }

        texts.push(TextLabel {
            rect: [
                card_x + 20.0 * scale,
                card_y + card_h - 86.0 * scale,
                card_w - 40.0 * scale,
                26.0 * scale,
            ],
            text: "Meld Guide — full visual reference".to_string(),
            color: color::STONE,
            align: TextAlign::Center,
            font_px: Some(13.0 * scale),
            ..Default::default()
        });

        let items = self.flat_items(w, h);
        use crate::render::theme::{ButtonState, ButtonVariant};
        use crate::ui::input::UiAction;
        let mut btn_quads = Vec::new();
        let mut junk_buttons = Vec::new();
        for item in &items {
            let (label, variant) = match item.action {
                SummaryAction::MeldGuide => ("Meld Guide", ButtonVariant::Default),
                SummaryAction::Continue => ("Continue", ButtonVariant::Primary),
            };
            crate::ui::widget::push_button(
                &mut btn_quads,
                &mut texts,
                &mut junk_buttons,
                crate::ui::widget::ButtonSpec {
                    rect: item.rect,
                    label,
                    variant,
                    state: ButtonState::Rest,
                    action: UiAction::Confirm,
                },
            );
        }
        frame.quads(btn_quads);
        frame.texts(texts);
        self.tree.register_flat_buttons(&items, &mut frame.buttons);
        frame.window_title = "Mahjuro — Tutorial Summary".to_string();
        frame
    }
}
