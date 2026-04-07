//! Pick-blind scene — Balatro-style: shows the next blind in the
//! Small → Big → Boss cycle. The player can play it, or skip it
//! (Small/Big only) to jump straight to the next blind.

use crate::core::rules::BlindKind;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::{ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx, relic_row};
use super::gameplay::GameplayScene;
use super::pause_menu::{PauseMenu, PauseUpdate};

pub struct PickBlindScene {
    /// Whether the skip button at the bottom is focused.
    skip_focused: bool,
    pause_menu: PauseMenu,
}

impl PickBlindScene {
    pub fn new() -> Self {
        Self { skip_focused: false, pause_menu: PauseMenu::new() }
    }

    fn can_skip(blind: BlindKind) -> bool {
        !matches!(blind, BlindKind::Boss)
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        // Pause menu handling.
        if self.pause_menu.paused {
            match self.pause_menu.update(
                ctx.actions,
                ctx.run,
                ctx.cursor_pos,
                ctx.layout.window_w,
                ctx.layout.window_h,
            ) {
                PauseUpdate::StayPaused | PauseUpdate::Resume => return None,
                PauseUpdate::Transition(t) => return t,
                PauseUpdate::Quit => {
                    *ctx.quit_requested = true;
                    return None;
                }
            }
        }

        let upcoming = ctx.run.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);

        // Mouse hover → focus. Layout must mirror draw().
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let hs = ctx.layout.hand_strip;
        let card_w = hs.w * 0.42;
        let pad_y = hs.h * 0.08;
        let card_h = hs.h - pad_y * 2.0;
        let card_x = hs.x + (hs.w - card_w) * 0.5;
        let card_y = hs.y + pad_y;
        let scale = (w.min(h)) / 600.0;
        let btn_w = (140.0 * scale).max(70.0);
        let btn_h = (32.0 * scale).max(20.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (12.0 * scale);
        let (cx, cy) = ctx.cursor_pos;
        let in_card =
            cx >= card_x && cx <= card_x + card_w && cy >= card_y && cy <= card_y + card_h;
        let in_skip = can_skip
            && cx >= btn_x
            && cx <= btn_x + btn_w
            && cy >= btn_y
            && cy <= btn_y + btn_h;
        if in_skip {
            self.skip_focused = true;
        } else if in_card {
            self.skip_focused = false;
        }

        for a in ctx.actions {
            match a {
                UiAction::FocusDown | UiAction::FocusNext => {
                    if can_skip {
                        self.skip_focused = true;
                    }
                }
                UiAction::FocusUp | UiAction::FocusPrev => {
                    self.skip_focused = false;
                }
                UiAction::Confirm | UiAction::CommitDiscard if self.skip_focused => {
                    if can_skip {
                        let reward = upcoming.skip_reward();
                        ctx.run.gold = ctx.run.gold.saturating_add(reward);
                        ctx.run.skip_to_next_blind();
                        return Some(Scene::PickBlind(PickBlindScene::new()));
                    }
                }
                UiAction::Confirm | UiAction::CommitDiscard => {
                    ctx.run.apply_blind(upcoming);
                    return Some(Scene::Gameplay(GameplayScene::new()));
                }
                UiAction::Cancel => {
                    if can_skip {
                        let reward = upcoming.skip_reward();
                        ctx.run.gold = ctx.run.gold.saturating_add(reward);
                        ctx.run.skip_to_next_blind();
                        return Some(Scene::PickBlind(PickBlindScene::new()));
                    }
                }
                UiAction::Pause => {
                    self.pause_menu.open();
                    return None;
                }
                _ => {}
            }
        }
        None
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let hs = ctx.layout.hand_strip;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.05, 0.08, 0.06, 1.0],
        }];

        // Relic row in its own strip.
        let (relic_insts, relic_labels, relic_icons) =
            relic_row(&ctx.run.relics, &ctx.layout.relic_strip, w);
        instances.extend(relic_insts);

        let upcoming = ctx.run.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);

        // Single centered card for the upcoming blind.
        let card_w = hs.w * 0.42;
        let pad_y = hs.h * 0.08;
        let card_h = hs.h - pad_y * 2.0;
        let card_x = hs.x + (hs.w - card_w) * 0.5;
        let card_y = hs.y + pad_y;

        let blind_color: [f32; 4] = match upcoming {
            BlindKind::Small => [0.2, 0.4, 0.6, 0.95],
            BlindKind::Big => [0.5, 0.35, 0.1, 0.95],
            BlindKind::Boss => [0.6, 0.15, 0.15, 0.95],
        };
        // Highlight ring around the card (always focused — it's the only choice).
        if !self.skip_focused {
            let pad = 4.0;
            instances.push(GpuInstance {
                rect: [card_x - pad, card_y - pad, card_w + pad * 2.0, card_h + pad * 2.0],
                color: [0.7, 0.55, 0.1, 1.0],
            });
        }
        instances.push(GpuInstance {
            rect: [card_x, card_y, card_w, card_h],
            color: blind_color,
        });

        // Skip button at bottom (Small/Big only).
        let scale = (w.min(h)) / 600.0;
        let btn_w = (140.0 * scale).max(70.0);
        let btn_h = (32.0 * scale).max(20.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (12.0 * scale);

        if can_skip {
            if self.skip_focused {
                let pad = 3.0;
                instances.push(GpuInstance {
                    rect: [btn_x - pad, btn_y - pad, btn_w + pad * 2.0, btn_h + pad * 2.0],
                    color: [0.9, 0.8, 0.2, 0.95],
                });
            }
            instances.push(GpuInstance {
                rect: [btn_x, btn_y, btn_w, btn_h],
                color: [0.5, 0.3, 0.1, 0.9],
            });
        }

        // Text labels.
        let ms = ctx.layout.modifier_strip;
        let base = ctx.run.base_target;
        let gold = ctx.run.gold;
        let effective_target = (base as f32 * upcoming.target_multiplier()) as u32;

        let mut text_labels = vec![
            TextLabel {
                rect: [
                    ctx.layout.score_panel.x,
                    ctx.layout.score_panel.y,
                    ctx.layout.score_panel.w,
                    ctx.layout.score_panel.h,
                ],
                text: format!(
                    "ANTE {}/{}   Gold: {}   Target: {}",
                    ctx.run.ante,
                    crate::game::run::FINAL_ANTE,
                    gold,
                    effective_target
                ),
                color: [1.0, 1.0, 1.0, 1.0],
            },
            TextLabel {
                rect: [ms.x, ms.y, ms.w, ms.h],
                text: format!(
                    "{}   |   Enter play   {}",
                    upcoming.description(),
                    if can_skip { "Esc/↓ skip" } else { "" }
                ),
                color: [1.0, 1.0, 1.0, 1.0],
            },
        ];
        text_labels.extend(relic_labels);

        // Card labels — dark text backdrop for readability, then text.
        let overlay_y = card_y + card_h * 0.22;
        let overlay_h = card_h * 0.62;
        instances.push(GpuInstance {
            rect: [card_x, overlay_y, card_w, overlay_h],
            color: [0.0, 0.0, 0.0, 0.45],
        });

        let name_h = card_h * 0.20;
        let name_y = card_y + card_h * 0.30;
        text_labels.push(TextLabel {
            rect: [card_x, name_y, card_w, name_h],
            text: upcoming.name().to_string(),
            color: [1.0, 1.0, 1.0, 1.0],
        });
        let mult = upcoming.target_multiplier();
        let desc_h = card_h * 0.15;
        let desc_y = card_y + card_h * 0.55;
        text_labels.push(TextLabel {
            rect: [card_x, desc_y, card_w, desc_h],
            text: format!("x{:.1} target", mult),
            color: [1.0, 1.0, 1.0, 0.95],
        });
        // Show forced modifier on Boss card.
        if let Some(modifier) = upcoming.forced_modifier(ctx.run.run_number) {
            let mod_h = card_h * 0.12;
            let mod_y = card_y + card_h * 0.72;
            text_labels.push(TextLabel {
                rect: [card_x, mod_y, card_w, mod_h],
                text: format!("{}: {}", modifier.name(), modifier.description()),
                color: [1.0, 0.75, 0.4, 1.0],
            });
        }

        let mut buttons = vec![ButtonDef::ui(
            (card_x, card_y, card_w, card_h),
            UiAction::Confirm,
        )];
        if can_skip {
            let reward = upcoming.skip_reward();
            text_labels.push(TextLabel {
                rect: [btn_x, btn_y, btn_w, btn_h],
                text: format!("Skip (+{}g)", reward),
                color: [1.0, 1.0, 1.0, 1.0],
            });
            buttons.push(ButtonDef::ui(
                (btn_x, btn_y, btn_w, btn_h),
                UiAction::Cancel,
            ));
        }

        // Pause overlay.
        self.pause_menu
            .draw(w, h, scale, &mut instances, &mut text_labels, &mut buttons);

        SceneDrawOutput {
            background: Default::default(),
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons,
            buttons,
            window_title: format!(
                "Next Blind: {}    Enter play  {}",
                upcoming.name(),
                if can_skip { "Esc skip" } else { "(must play)" },
            ),
            departing_indices: vec![],
            hint_indices: vec![],
        }
    }
}
