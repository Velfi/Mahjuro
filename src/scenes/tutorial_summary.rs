//! End-of-tutorial summary shown after the onboarding finale.

use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::persistence;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{color, typography};
use crate::render::vocabulary_colors::GlossaryMode;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::sfx_id::SfxId;
use crate::ui::controller_hints::{
    HintStyle, menu_footer_row, push_screen_footer_hint, screen_footer_reserve,
};
use crate::ui::focus_nav;
use crate::ui::styled_text::styled_line_block_height_at_font_px;
use crate::ui::widget::{self, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use crate::game::onboarding_intro_copy;
use super::guide::GuideScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SummaryAction {
    Continue,
    Guide,
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
        let mut tree = TreeState::new();
        tree.set_focus(SummaryAction::Continue.id());
        Self { won, tree }
    }

    fn flat_items(&self, w: f32, h: f32) -> Vec<FlatItem<SummaryAction>> {
        let scale = (w / 800.0).min(h / 600.0).max(0.5);
        let btn_w = (200.0 * scale).min(w * 0.38).max(140.0);
        let btn_h = (44.0 * scale).max(32.0);
        let gap = 16.0 * scale;
        let y = h - screen_footer_reserve(w, h) - btn_h - 28.0 * scale;
        let pair_w = btn_w * 2.0 + gap;
        let start_x = (w - pair_w) * 0.5;
        vec![
            FlatItem::new(
                SummaryAction::Guide.id(),
                [start_x, y, btn_w, btn_h],
                SummaryAction::Guide,
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
            Some(SummaryAction::Guide) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                *ctx.overlay_request = Some(super::OverlayRequest::Push(Box::new(Scene::Guide(
                    GuideScene::new(),
                ))));
                None
            }
            Some(SummaryAction::Continue) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                let settings = persistence::load_settings();
                GameEngine::reset_to_demo(ctx.run, ctx.progress, &settings);
                Some(SceneIntent::MainMenu)
            }
            None => None,
        }
    }

    fn draw_frame(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
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
            "You surpassed The Iconoclast and won, an auspicious beginning."
        } else {
            "You reached the finale but faltered against The Iconoclast. Perhaps you'll fare better next time."
        };
        let bullets = onboarding_intro_copy::SUMMARY_BULLETS;

        let mut frame = UiFrame::new();
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::WALNUT_INK,
            user: 0,
        });
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        if self.won && ctx.effect_layers.golden_dust {
            frame.golden_dust();
        }

        let card_w = (620.0 * scale).min(w * 0.88);
        let card_h = (480.0 * scale).min(h * 0.82);
        let card_x = (w - card_w) * 0.5;
        let card_y = (h - card_h) * 0.5;
        let border = (2.0 * scale).max(1.0);

        frame.quad(GpuInstance {
            rect: [card_x, card_y, card_w, card_h],
            color: color::WALNUT_DEEP,
            user: 0,
        });
        frame.quad(GpuInstance {
            rect: [card_x, card_y, card_w, border],
            color: color::BRASS,
            user: 0,
        });
        frame.quad(GpuInstance {
            rect: [card_x, card_y + card_h - border, card_w, border],
            color: color::BRASS,
            user: 0,
        });
        frame.quad(GpuInstance {
            rect: [card_x, card_y, border, card_h],
            color: color::BRASS,
            user: 0,
        });
        frame.quad(GpuInstance {
            rect: [card_x + card_w - border, card_y, border, card_h],
            color: color::BRASS,
            user: 0,
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
            font_px: Some(typography::size(typography::H28, h)),
            ..Default::default()
        });

        let subtitle_x = card_x + 34.0 * scale;
        let subtitle_y = card_y + 78.0 * scale;
        let subtitle_w = card_w - 68.0 * scale;
        let subtitle_font = typography::size(typography::H36, h);
        let subtitle_h = styled_line_block_height_at_font_px(
            subtitle,
            subtitle_w,
            subtitle_font,
            GlossaryMode::Prose,
            color::PARCHMENT,
        );
        widget::push_text_block(
            &mut texts,
            [subtitle_x, subtitle_y, subtitle_w, subtitle_h],
            subtitle,
            TextStyle {
                tier: typography::H36,
                color: color::PARCHMENT,
                padding: 0.0,
                align: TextAlign::Center,
                glossary: GlossaryMode::Prose,
            },
            h,
        );

        let mut bullet_y = subtitle_y + subtitle_h + 20.0 * scale;
        for bullet in bullets {
            let bullet_text = format!("• {bullet}");
            let bullet_w = card_w - 72.0 * scale;
            let bullet_font = typography::size(typography::H36, h);
            let bullet_h = styled_line_block_height_at_font_px(
                &bullet_text,
                bullet_w,
                bullet_font,
                GlossaryMode::Prose,
                color::STONE,
            );
            widget::push_text_block(
                &mut texts,
                [card_x + 36.0 * scale, bullet_y, bullet_w, bullet_h],
                &bullet_text,
                TextStyle {
                    tier: typography::H36,
                    color: color::STONE,
                    padding: 0.0,
                    align: TextAlign::Left,
                    glossary: GlossaryMode::Prose,
                },
                h,
            );
            bullet_y += bullet_h + 10.0 * scale;
        }

        let items = self.flat_items(w, h);
        use crate::render::theme::{ButtonState, ButtonVariant};
        use crate::ui::input::UiAction;
        let mut btn_quads = Vec::new();
        let mut junk_buttons = Vec::new();
        for item in &items {
            let focused = self.tree.focused() == Some(item.id);
            let (label, variant, state) = match item.action {
                SummaryAction::Guide => (
                    "Guide",
                    ButtonVariant::Default,
                    if focused {
                        ButtonState::Hover
                    } else {
                        ButtonState::Rest
                    },
                ),
                SummaryAction::Continue => (
                    "Continue",
                    ButtonVariant::Primary,
                    if focused {
                        ButtonState::Hover
                    } else {
                        ButtonState::Rest
                    },
                ),
            };
            crate::ui::widget::push_button(
                &mut btn_quads,
                &mut texts,
                &mut junk_buttons,
                crate::ui::widget::ButtonSpec {
                    rect: item.rect,
                    label,
                    variant,
                    state,
                    action: UiAction::Confirm,
                },
            );
            if focused {
                focus_nav::push_focus_ring(item.rect, scale, w, h, &mut btn_quads);
            }
        }
        junk_buttons.clear();
        frame.quads(btn_quads);
        frame.texts(texts);
        self.tree.register_flat_buttons(&items, &mut frame.buttons);
        push_screen_footer_hint(
            &mut frame,
            &ctx,
            menu_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );
        frame.window_title = "Mahjuro — Tutorial Summary".to_string();
        ctx.stash_focus_nav_tree_flat(&self.tree, &items, |a| match a {
            SummaryAction::Guide => "Guide".into(),
            SummaryAction::Continue => "Continue".into(),
        });
        frame
    }
}
