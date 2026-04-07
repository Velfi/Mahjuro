//! Shop scene — between rounds; player can buy relics with gold.

use rand::seq::SliceRandom;

use crate::core::relic::{
    Rarity, RelicId, RelicState, all_relic_defs, relic_buy_price as relic_price,
    relic_sell_price,
};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;

use super::{ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx, relic_badge_rect, relic_row};
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

    /// Evenly-spaced card rects within the hand strip area.
    /// Same layout used by both update() (hit-testing) and draw() (rendering).
    fn card_rects(&self, layout: &LayoutResult) -> Vec<(f32, f32, f32, f32)> {
        let hs = layout.hand_strip;
        let n = self.items.len();
        if n == 0 {
            return vec![];
        }
        let gap_frac = 0.04;
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
    }

    /// Screen rect of the "Next Round" button.
    fn next_round_rect(layout: &LayoutResult) -> (f32, f32, f32, f32) {
        let w = layout.window_w;
        let h = layout.window_h;
        let scale = (w.min(h)) / 600.0;
        let btn_w = (160.0 * scale).max(80.0);
        let btn_h = (36.0 * scale).max(22.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (16.0 * scale);
        (btn_x, btn_y, btn_w, btn_h)
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

        // Cursor space layout:
        //   0..n_items                          → buyable shop items
        //   n_items..n_items+n_owned            → owned relics (sell)
        //   n_items+n_owned                     → "Next Round" button
        let n_items = self.items.len();
        let n_owned = ctx.run.relics.active.len();
        let total = n_items + n_owned;

        // Mouse hover → focus. Hit-test cursor against shop cards, owned relic
        // badges, and the Next Round button so the keyboard cursor follows the
        // mouse and clicks act on whatever's under the pointer.
        let (cx, cy) = ctx.cursor_pos;
        let hit = |r: (f32, f32, f32, f32)| {
            cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3
        };
        let cards = self.card_rects(ctx.layout);
        let mut hovered: Option<usize> = None;
        for (i, r) in cards.iter().enumerate() {
            if hit(*r) {
                hovered = Some(i);
                break;
            }
        }
        if hovered.is_none() {
            for i in 0..n_owned {
                let r = relic_badge_rect(
                    &ctx.layout.relic_strip,
                    ctx.layout.window_w,
                    ctx.run.relics.max_slots,
                    i,
                );
                if hit(r) {
                    hovered = Some(n_items + i);
                    break;
                }
            }
        }
        if hovered.is_none() && hit(Self::next_round_rect(ctx.layout)) {
            hovered = Some(total);
        }
        if let Some(h) = hovered {
            self.cursor = h;
        }

        for a in ctx.actions {
            match a {
                UiAction::FocusNext => {
                    self.cursor = (self.cursor + 1).min(total);
                }
                UiAction::FocusPrev => {
                    self.cursor = self.cursor.saturating_sub(1);
                }
                UiAction::FocusDown => {
                    // Jump to the "Next Round" button.
                    self.cursor = total;
                }
                UiAction::FocusUp => {
                    // Jump back from the button onto the last selectable thing.
                    if self.cursor >= total && total > 0 {
                        self.cursor = total - 1;
                    }
                }
                UiAction::CommitDiscard => {
                    // "Next Round" button (mouse click or Enter key) — always proceed.
                    return Some(Scene::PickBlind(PickBlindScene::new()));
                }
                UiAction::Confirm => {
                    if self.cursor >= total {
                        return Some(Scene::PickBlind(PickBlindScene::new()));
                    }
                    if self.cursor >= n_items {
                        // Sell the focused owned relic.
                        let idx = self.cursor - n_items;
                        if idx < ctx.run.relics.active.len() {
                            let rid = ctx.run.relics.active[idx];
                            let refund = relic_sell_price(rid);
                            ctx.run.relics.active.remove(idx);
                            ctx.run.gold = ctx.run.gold.saturating_add(refund);
                            // Re-clamp cursor: total shrank by 1.
                            let new_total = n_items + ctx.run.relics.active.len();
                            self.cursor = self.cursor.min(new_total);
                        }
                    } else {
                        // Buy the focused shop item.
                        let item = &mut self.items[self.cursor];
                        if !item.sold && ctx.run.gold >= item.price && !ctx.run.relics.is_full() {
                            ctx.run.gold -= item.price;
                            ctx.run.relics.active.push(item.relic);
                            item.sold = true;
                        }
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

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.08, 0.06, 0.12, 1.0],
        }];

        // Cursor space (must match update()):
        //   0..n_items                          → shop items
        //   n_items..n_items+n_owned            → owned relics
        //   n_items+n_owned                     → "Next Round" button
        let n_items = self.items.len();
        let n_owned = ctx.run.relics.active.len();
        let total = n_items + n_owned;

        // Gold border under the focused owned relic (drawn before relic_row
        // so the badge background sits on top of it like a card border).
        if self.cursor >= n_items && self.cursor < total {
            let owned_idx = self.cursor - n_items;
            let (rx, ry, rw, rh) =
                relic_badge_rect(&ctx.layout.relic_strip, w, ctx.run.relics.max_slots, owned_idx);
            let pad = 3.0;
            instances.push(GpuInstance {
                rect: [rx - pad, ry - pad, rw + pad * 2.0, rh + pad * 2.0],
                color: [0.9, 0.8, 0.2, 0.95],
            });
        }

        // Relic row in its own strip.
        let (relic_insts, relic_labels, relic_icons) = relic_row(&ctx.run.relics, &ctx.layout.relic_strip, w);
        instances.extend(relic_insts);

        // Evenly-spaced card rects within the hand strip area (shared with update()).
        let card_rects = self.card_rects(ctx.layout);

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

        // "Next Round" button (rect shared with update() hit-testing).
        let scale = (w.min(h)) / 600.0;
        let (btn_x, btn_y, btn_w, btn_h) = Self::next_round_rect(ctx.layout);
        let next_focused = self.cursor >= total;
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
        let desc_text = if self.cursor < n_items {
            let item = &self.items[self.cursor];
            if item.sold {
                format!("{} — SOLD", item.name)
            } else {
                item.description.to_string()
            }
        } else if self.cursor < total {
            let owned_idx = self.cursor - n_items;
            let rid = ctx.run.relics.active[owned_idx];
            let defs = all_relic_defs();
            let def = defs.iter().find(|d| d.id == rid);
            let (name, desc) = def
                .map(|d| (d.name, d.description))
                .unwrap_or(("Relic", ""));
            format!(
                "{}: {}  —  Sell for {}g (Space)",
                name,
                desc,
                relic_sell_price(rid)
            )
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

        let mut buttons = vec![ButtonDef::ui(
            (btn_x, btn_y, btn_w, btn_h),
            UiAction::CommitDiscard,
        )];

        // Make each shop card clickable. Hover-focus in update() ensures
        // self.cursor already points at the hovered card by the time the
        // Confirm action is processed.
        for &(cx, cy, cw, ch) in &card_rects {
            buttons.push(ButtonDef::ui((cx, cy, cw, ch), UiAction::Confirm));
        }

        // Make each owned relic badge clickable for sale.
        for i in 0..n_owned {
            buttons.push(ButtonDef::ui(
                relic_badge_rect(&ctx.layout.relic_strip, w, ctx.run.relics.max_slots, i),
                UiAction::Confirm,
            ));
        }

        // Sell pill on the focused owned relic — visible button + clickable.
        // Uses UiAction::Confirm so the button click takes the same code path
        // as pressing Space while the relic is focused.
        if self.cursor >= n_items && self.cursor < total {
            let owned_idx = self.cursor - n_items;
            let rid = ctx.run.relics.active[owned_idx];
            let (rx, ry, rw, rh) =
                relic_badge_rect(&ctx.layout.relic_strip, w, ctx.run.relics.max_slots, owned_idx);
            let pill_h = rh * 0.30;
            let pill_w = rw * 0.88;
            let pill_x = rx + (rw - pill_w) * 0.5;
            let pill_y = ry + rh - pill_h - rh * 0.04;
            // Pill border (dark) for contrast against the gold fill.
            let border = 1.5;
            instances.push(GpuInstance {
                rect: [
                    pill_x - border,
                    pill_y - border,
                    pill_w + border * 2.0,
                    pill_h + border * 2.0,
                ],
                color: [0.05, 0.05, 0.1, 0.95],
            });
            instances.push(GpuInstance {
                rect: [pill_x, pill_y, pill_w, pill_h],
                color: [0.95, 0.78, 0.22, 0.98],
            });
            text_labels.push(TextLabel {
                rect: [pill_x, pill_y, pill_w, pill_h],
                text: format!("Sell {}g", relic_sell_price(rid)),
                color: [0.08, 0.06, 0.12, 1.0],
            });
            buttons.push(ButtonDef::ui(
                (pill_x, pill_y, pill_w, pill_h),
                UiAction::Confirm,
            ));
        }

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
                "Mahjuro — Shop (Round {}) — Gold: {} — ←→ browse  Space buy/sell  Enter next round",
                self.came_from_round, ctx.run.gold
            ),
            departing_indices: vec![],
            hint_indices: vec![],
        }
    }
}
