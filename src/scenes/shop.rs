//! Shop scene — between rounds; player can buy relics with gold.

use rand::seq::SliceRandom;

use crate::core::relic::{Rarity, RelicId, RelicState, all_relic_defs};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;

use super::{ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx, relic_row};
use super::pause_menu::{PauseMenu, PauseUpdate};
use super::pick_blind::PickBlindScene;

/// A purchasable item in the shop.
struct ShopItem {
    relic: RelicId,
    name: &'static str,
    description: &'static str,
    rarity: Rarity,
    price: u32,
    sold: bool,
}

pub struct ShopScene {
    pub came_from_round: u32,
    items: Vec<ShopItem>,
    cursor: usize,
    pause_menu: PauseMenu,
}

/// Gold cost for a relic (simple formula based on position in the defs list).
fn relic_price(id: RelicId) -> u32 {
    let defs = all_relic_defs();
    let idx = defs.iter().position(|d| d.id == id).unwrap_or(0);
    4 + (idx as u32 % 5)
}

impl ShopScene {
    pub fn new(came_from_round: u32, relics: &RelicState) -> Self {
        let defs = all_relic_defs();
        let mut pool: Vec<&_> = defs.iter().filter(|d| !relics.has(d.id)).collect();
        pool.shuffle(&mut rand::rng());

        let items: Vec<ShopItem> = pool
            .into_iter()
            .take(3)
            .map(|d| ShopItem {
                relic: d.id,
                name: d.name,
                description: d.description,
                rarity: d.rarity,
                price: relic_price(d.id),
                sold: false,
            })
            .collect();

        Self {
            came_from_round,
            items,
            cursor: 0,
            pause_menu: PauseMenu::new(),
        }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        // Pause menu handling.
        if self.pause_menu.paused {
            match self.pause_menu.update(ctx.actions, ctx.run) {
                PauseUpdate::StayPaused | PauseUpdate::Resume => return None,
                PauseUpdate::Transition(t) => return t,
                PauseUpdate::Quit => {
                    *ctx.quit_requested = true;
                    return None;
                }
            }
        }

        for a in ctx.actions {
            match a {
                UiAction::FocusNext => {
                    self.cursor = (self.cursor + 1).min(self.items.len());
                }
                UiAction::FocusPrev => {
                    self.cursor = self.cursor.saturating_sub(1);
                }
                UiAction::FocusDown => {
                    // Jump to the "Next Round" button.
                    self.cursor = self.items.len();
                }
                UiAction::FocusUp => {
                    // Jump back to items from the button.
                    if self.cursor >= self.items.len() && !self.items.is_empty() {
                        self.cursor = self.items.len() - 1;
                    }
                }
                UiAction::CommitDiscard => {
                    // "Next Round" button (mouse click or Enter key).
                    if self.cursor >= self.items.len() {
                        return Some(Scene::PickBlind(PickBlindScene::new()));
                    }
                    // Keyboard Enter when cursor is on an item — move cursor
                    // to the "Next Round" position and confirm on next press,
                    // unless this was a direct button click (always proceed).
                    return Some(Scene::PickBlind(PickBlindScene::new()));
                }
                UiAction::Confirm => {
                    if self.cursor >= self.items.len() {
                        return Some(Scene::PickBlind(PickBlindScene::new()));
                    }
                    let item = &mut self.items[self.cursor];
                    if !item.sold && ctx.run.gold >= item.price && !ctx.run.relics.is_full() {
                        ctx.run.gold -= item.price;
                        ctx.run.relics.active.push(item.relic);
                        item.sold = true;
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
        let sp = ctx.layout.score_panel;
        let ms = ctx.layout.modifier_strip;
        let hs = ctx.layout.hand_strip;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.08, 0.06, 0.12, 1.0],
        }];

        // Relic row in its own strip.
        let (relic_insts, relic_labels, relic_icons) = relic_row(&ctx.run.relics, &ctx.layout.relic_strip, w);
        instances.extend(relic_insts);

        // Evenly-spaced card rects within the hand strip area.
        let n = self.items.len();
        let card_rects: Vec<(f32, f32, f32, f32)> = if n == 0 {
            vec![]
        } else {
            let gap_frac = 0.04; // 4% of strip width per gap
            let outer_pad = hs.w * gap_frac;
            let inner_gap = hs.w * gap_frac;
            let total_gaps = outer_pad * 2.0 + inner_gap * (n as f32 - 1.0).max(0.0);
            let card_w = (hs.w - total_gaps) / n as f32;
            let pad_y = hs.h * 0.08;
            let card_h = hs.h - pad_y * 2.0;
            let card_y = hs.y + pad_y;
            (0..n)
                .map(|i| {
                    let card_x = hs.x + outer_pad + i as f32 * (card_w + inner_gap);
                    (card_x, card_y, card_w, card_h)
                })
                .collect()
        };

        // Rarity accent color for card top stripe.
        fn rarity_accent(rarity: Rarity) -> [f32; 4] {
            match rarity {
                Rarity::Common => [0.5, 0.5, 0.5, 0.9],
                Rarity::Uncommon => [0.2, 0.7, 0.2, 0.9],
                Rarity::Rare => [0.25, 0.45, 1.0, 0.9],
                Rarity::Legendary => [1.0, 0.75, 0.15, 0.9],
            }
        }

        for (i, &(cx, cy, cw, ch)) in card_rects.iter().enumerate() {
            let item = &self.items[i];
            let focused = i == self.cursor;
            let can_afford = ctx.run.gold >= item.price && !ctx.run.relics.is_full();

            // Gold highlight border around focused card.
            if focused && !item.sold {
                let pad = 3.0;
                instances.push(GpuInstance {
                    rect: [cx - pad, cy - pad, cw + pad * 2.0, ch + pad * 2.0],
                    color: [0.9, 0.8, 0.2, 0.95],
                });
            }

            // Card background — dim if sold or unaffordable.
            let color = if item.sold {
                [0.15, 0.15, 0.15, 0.5]
            } else if !can_afford {
                [0.18, 0.18, 0.25, 0.85]
            } else {
                [0.15, 0.22, 0.38, 0.92]
            };
            instances.push(GpuInstance {
                rect: [cx, cy, cw, ch],
                color,
            });

            // Rarity-colored accent stripe at card top.
            if !item.sold {
                let stripe_h = ch * 0.04;
                instances.push(GpuInstance {
                    rect: [cx, cy, cw, stripe_h],
                    color: rarity_accent(item.rarity),
                });
            }

            // Dark text backdrop for readability (like pick_blind).
            let overlay_y = cy + ch * 0.18;
            let overlay_h = ch * 0.70;
            instances.push(GpuInstance {
                rect: [cx, overlay_y, cw, overlay_h],
                color: [0.0, 0.0, 0.0, 0.4],
            });
        }

        // "Next Round" button.
        let scale = (w.min(h)) / 600.0;
        let btn_w = (160.0 * scale).max(80.0);
        let btn_h = (36.0 * scale).max(22.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (16.0 * scale);
        let next_focused = self.cursor >= self.items.len();
        let next_color = if next_focused {
            [0.2, 0.6, 0.3, 0.95]
        } else {
            [0.22, 0.38, 0.55, 0.92]
        };
        if next_focused {
            let pad = 3.0;
            instances.push(GpuInstance {
                rect: [btn_x - pad, btn_y - pad, btn_w + pad * 2.0, btn_h + pad * 2.0],
                color: [0.9, 0.8, 0.2, 0.95],
            });
        }
        instances.push(GpuInstance {
            rect: [btn_x, btn_y, btn_w, btn_h],
            color: next_color,
        });

        // Text labels.
        let mut text_labels = vec![
            TextLabel {
                rect: [sp.x, sp.y, sp.w, sp.h],
                text: format!("SHOP  —  Round {}   Gold: {}", self.came_from_round, ctx.run.gold),
                color: [1.0, 1.0, 1.0, 1.0],
            },
        ];
        text_labels.extend(relic_labels);

        // Description of selected item in modifier strip.
        let desc_text = if self.cursor < self.items.len() {
            let item = &self.items[self.cursor];
            if item.sold {
                format!("{} — SOLD", item.name)
            } else {
                item.description.to_string()
            }
        } else {
            "Leave shop and pick your next blind".into()
        };
        text_labels.push(TextLabel {
            rect: [ms.x, ms.y, ms.w, ms.h],
            text: desc_text,
            color: [0.9, 0.85, 0.6, 1.0],
        });

        // Card labels — name, price, rarity, and affordability.
        for (i, &(cx, cy, cw, ch)) in card_rects.iter().enumerate() {
            let item = &self.items[i];
            let can_afford = ctx.run.gold >= item.price && !ctx.run.relics.is_full();

            // Rarity label near top of card (below the accent stripe).
            let rarity_h = ch * 0.10;
            let rarity_y = cy + ch * 0.06;
            let (rarity_text, rarity_color) = match item.rarity {
                Rarity::Common => ("Common", [0.7, 0.7, 0.7, 0.9]),
                Rarity::Uncommon => ("Uncommon", [0.3, 0.8, 0.3, 0.9]),
                Rarity::Rare => ("Rare", [0.3, 0.5, 1.0, 0.9]),
                Rarity::Legendary => ("Legendary", [1.0, 0.8, 0.2, 0.9]),
            };
            text_labels.push(TextLabel {
                rect: [cx, rarity_y, cw, rarity_h],
                text: rarity_text.to_string(),
                color: if item.sold { [0.4, 0.4, 0.4, 0.6] } else { rarity_color },
            });

            // Name in upper-center area of card.
            let name_h = ch * 0.18;
            let name_y = cy + ch * 0.22;
            text_labels.push(TextLabel {
                rect: [cx, name_y, cw, name_h],
                text: item.name.to_string(),
                color: if item.sold { [0.5, 0.5, 0.5, 0.7] } else { [1.0, 1.0, 1.0, 1.0] },
            });

            // Description on the card (sized for readability, not raw card rect).
            let desc_rect_h = ch * 0.22;
            let desc_rect_w = cw * 0.88;
            let desc_x = cx + (cw - desc_rect_w) * 0.5;
            let desc_y = cy + ch * 0.44;
            text_labels.push(TextLabel {
                rect: [desc_x, desc_y, desc_rect_w, desc_rect_h],
                text: item.description.to_string(),
                color: if item.sold {
                    [0.4, 0.4, 0.4, 0.6]
                } else {
                    [0.8, 0.78, 0.65, 0.95]
                },
            });

            // Price / SOLD label in lower area.
            let price_h = ch * 0.15;
            let price_y = cy + ch * 0.72;
            if item.sold {
                text_labels.push(TextLabel {
                    rect: [cx, price_y, cw, price_h],
                    text: "SOLD".to_string(),
                    color: [0.6, 0.6, 0.6, 0.7],
                });
            } else {
                let price_color = if can_afford {
                    [1.0, 0.85, 0.3, 1.0]
                } else {
                    [0.9, 0.3, 0.3, 1.0]
                };
                text_labels.push(TextLabel {
                    rect: [cx, price_y, cw, price_h],
                    text: format!("{}g", item.price),
                    color: price_color,
                });
            }

            // "Can't afford" indicator for unaffordable items.
            if !item.sold && !can_afford {
                let warn_h = ch * 0.10;
                let warn_y = cy + ch * 0.88;
                let reason = if ctx.run.relics.is_full() {
                    "Relics full"
                } else {
                    "Not enough gold"
                };
                text_labels.push(TextLabel {
                    rect: [cx, warn_y, cw, warn_h],
                    text: reason.to_string(),
                    color: [0.8, 0.3, 0.3, 0.85],
                });
            }
        }

        text_labels.push(TextLabel {
            rect: [btn_x, btn_y, btn_w, btn_h],
            text: "Next Round".into(),
            color: [1.0, 1.0, 1.0, 1.0],
        });

        let mut buttons = vec![
            ButtonDef {
                rect: (btn_x, btn_y, btn_w, btn_h),
                action: UiAction::CommitDiscard,
            },
        ];

        // Pause overlay.
        self.pause_menu.draw(w, h, scale, &mut instances, &mut text_labels, &mut buttons);

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
                "Mahjuro — Shop (Round {}) — Gold: {} — ←→ browse  Space buy  Enter next round",
                self.came_from_round, ctx.run.gold
            ),
            departing_indices: vec![],
            hint_indices: vec![],
        }
    }
}
