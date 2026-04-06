//! Pick-blind scene — player chooses which blind to attempt or skips.

use crate::core::rules::BlindKind;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::{ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx, relic_row};
use super::gameplay::GameplayScene;
use super::shop::ShopScene;

pub struct PickBlindScene {
    pub cursor: usize,
}

const BLINDS: [BlindKind; 3] = [BlindKind::Small, BlindKind::Big, BlindKind::Boss];

/// Gold reward for skipping a blind.
fn skip_reward(blind: BlindKind) -> u32 {
    match blind {
        BlindKind::Small => 3,
        BlindKind::Big => 5,
        BlindKind::Boss => 0,
    }
}

impl PickBlindScene {
    pub fn new() -> Self {
        Self { cursor: 0 }
    }

    fn can_skip(&self) -> bool {
        !matches!(BLINDS[self.cursor], BlindKind::Boss)
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for a in ctx.actions {
            match a {
                UiAction::FocusNext => self.cursor = (self.cursor + 1).min(2),
                UiAction::FocusPrev => self.cursor = self.cursor.saturating_sub(1),
                UiAction::Confirm | UiAction::CommitDiscard => {
                    let blind = BLINDS[self.cursor];
                    ctx.run.apply_blind(blind);
                    return Some(Scene::Gameplay(GameplayScene::new()));
                }
                UiAction::Cancel => {
                    if self.can_skip() {
                        let blind = BLINDS[self.cursor];
                        let reward = skip_reward(blind);
                        ctx.run.gold = ctx.run.gold.saturating_add(reward);
                        ctx.run.base_target = (ctx.run.base_target as f32 * 1.2) as u32;
                        ctx.run.run_number += 1;
                        return Some(Scene::Shop(ShopScene::new(
                            ctx.run.run_number,
                            &ctx.run.relics,
                        )));
                    }
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

        // Relic row below score panel.
        let sp = ctx.layout.score_panel;
        let (relic_insts, relic_labels, relic_icons) = relic_row(&ctx.run.relics, &sp, w);
        instances.extend(relic_insts);

        // Evenly-spaced card rects for the 3 blinds.
        let n = BLINDS.len();
        let gap_frac = 0.04;
        let outer_pad = hs.w * gap_frac;
        let inner_gap = hs.w * gap_frac;
        let total_gaps = outer_pad * 2.0 + inner_gap * (n as f32 - 1.0);
        let card_w = (hs.w - total_gaps) / n as f32;
        let pad_y = hs.h * 0.08;
        let card_h = hs.h - pad_y * 2.0;
        let card_y = hs.y + pad_y;
        let card_rects: Vec<(f32, f32, f32, f32)> = (0..n)
            .map(|i| {
                let card_x = hs.x + outer_pad + i as f32 * (card_w + inner_gap);
                (card_x, card_y, card_w, card_h)
            })
            .collect();

        let blind_colors: [[f32; 4]; 3] = [
            [0.2, 0.4, 0.6, 0.95],
            [0.5, 0.35, 0.1, 0.95],
            [0.6, 0.15, 0.15, 0.95],
        ];

        for (i, &(cx, cy, cw, ch)) in card_rects.iter().enumerate() {
            let color = if i == self.cursor {
                [0.9, 0.75, 0.2, 1.0]
            } else {
                blind_colors[i]
            };
            instances.push(GpuInstance {
                rect: [cx, cy, cw, ch],
                color,
            });
        }

        // Skip button at bottom (Small/Big only).
        let scale = (w.min(h)) / 600.0;
        let btn_w = (140.0 * scale).max(70.0);
        let btn_h = (32.0 * scale).max(20.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (12.0 * scale);

        let can_skip = self.can_skip();
        if can_skip {
            instances.push(GpuInstance {
                rect: [btn_x, btn_y, btn_w, btn_h],
                color: [0.5, 0.3, 0.1, 0.9],
            });
        }

        // Text labels.
        let ms = ctx.layout.modifier_strip;
        let base = ctx.run.base_target;
        let gold = ctx.run.gold;
        let selected_blind = BLINDS[self.cursor];
        let effective_target = (base as f32 * selected_blind.target_multiplier()) as u32;

        let mut text_labels = vec![
            TextLabel {
                rect: [sp.x, sp.y, sp.w, sp.h],
                text: format!(
                    "CHOOSE YOUR BLIND   Gold: {}   Target: {}",
                    gold, effective_target
                ),
                color: [1.0, 1.0, 1.0, 1.0],
            },
            TextLabel {
                rect: [ms.x, ms.y, ms.w, ms.h],
                text: format!(
                    "{}   |   ←→ choose   Enter play   {}",
                    selected_blind.description(),
                    if can_skip { "Esc skip" } else { "" }
                ),
                color: [1.0, 1.0, 1.0, 1.0],
            },
        ];
        text_labels.extend(relic_labels);

        // Card labels — blind name centered, multiplier below.
        for (i, &(cx, cy, cw, ch)) in card_rects.iter().enumerate() {
            let name_h = ch * 0.20;
            let name_y = cy + ch * 0.30;
            text_labels.push(TextLabel {
                rect: [cx, name_y, cw, name_h],
                text: BLINDS[i].name().to_string(),
                color: [1.0, 1.0, 1.0, 1.0],
            });
            let mult = BLINDS[i].target_multiplier();
            let desc_h = ch * 0.15;
            let desc_y = cy + ch * 0.65;
            text_labels.push(TextLabel {
                rect: [cx, desc_y, cw, desc_h],
                text: format!("x{:.1} target", mult),
                color: [0.8, 0.8, 0.8, 0.9],
            });
        }

        let mut buttons = vec![];
        if can_skip {
            let reward = skip_reward(BLINDS[self.cursor]);
            text_labels.push(TextLabel {
                rect: [btn_x, btn_y, btn_w, btn_h],
                text: format!("Skip (+{}g)", reward),
                color: [1.0, 1.0, 1.0, 1.0],
            });
            buttons.push(ButtonDef {
                rect: (btn_x, btn_y, btn_w, btn_h),
                action: UiAction::Cancel,
            });
        }

        let fmt = |i: usize| -> String {
            let blind = BLINDS[i];
            if self.cursor == i {
                format!("<<{}>>", blind.name())
            } else {
                format!("[{}]", blind.name())
            }
        };

        SceneDrawOutput {
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons,
            buttons,
            window_title: format!(
                "Pick Blind:  {}  {}  {}    ←→ choose  Enter play  Esc skip",
                fmt(0),
                fmt(1),
                fmt(2),
            ),
        }
    }
}
