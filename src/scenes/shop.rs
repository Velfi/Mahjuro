//! Shop scene — between rounds; player can buy relics with gold.

use rand::seq::SliceRandom;

use crate::core::relic::{
    Rarity, RelicId, RelicState, all_relic_defs, relic_buy_price as relic_price, relic_sell_price,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;
use crate::ui::widget::{self, PanelVariant, TextStyle};

use super::pause_menu::{PauseMenu, PauseUpdate};
use super::pick_blind::PickBlindScene;
use super::{
    ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx, relic_badge_rect,
    relic_row,
};

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
        let hit =
            |r: (f32, f32, f32, f32)| cx >= r.0 && cx <= r.0 + r.2 && cy >= r.1 && cy <= r.1 + r.3;
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

        // Background — Midnight Gold deep base.
        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        }];

        // Cursor space (must match update()):
        //   0..n_items                          → shop items
        //   n_items..n_items+n_owned            → owned relics
        //   n_items+n_owned                     → "Next Round" button
        let n_items = self.items.len();
        let n_owned = ctx.run.relics.active.len();
        let total = n_items + n_owned;

        // Gold border under the focused owned relic.
        if self.cursor >= n_items && self.cursor < total {
            let owned_idx = self.cursor - n_items;
            let (rx, ry, rw, rh) = relic_badge_rect(
                &ctx.layout.relic_strip,
                w,
                ctx.run.relics.max_slots,
                owned_idx,
            );
            let pad = 3.0;
            instances.push(GpuInstance {
                rect: [rx - pad, ry - pad, rw + pad * 2.0, rh + pad * 2.0],
                color: color::GOLD,
            });
        }

        // Relic row in its own strip.
        let (relic_insts, relic_labels, relic_icons) =
            relic_row(&ctx.run.relics, &ctx.layout.relic_strip, w);
        instances.extend(relic_insts);

        // Evenly-spaced card rects within the hand strip area (shared with update()).
        let card_rects = self.card_rects(ctx.layout);

        // Rarity accent color for card top stripe — pulled from the central
        // theme so the shop and collection scenes don't drift apart.
        fn rarity_accent(rarity: Rarity) -> [f32; 4] {
            match rarity {
                Rarity::Common => color::rarity(0),
                Rarity::Uncommon => color::rarity(1),
                Rarity::Rare => color::rarity(2),
                Rarity::Legendary => color::rarity(3),
            }
        }

        let mut text_labels: Vec<TextLabel> = Vec::new();

        for (i, &(cx, cy, cw, ch)) in card_rects.iter().enumerate() {
            let item = &self.items[i];
            let focused = i == self.cursor;
            let can_afford = ctx.run.gold >= item.price && !ctx.run.relics.is_full();

            // Hover-lift: focused card sits ~12px higher (or 2% of card height,
            // whichever is larger). Static lift for now — no tween — so the
            // movement reads instantly when the cursor moves between cards.
            let lift = if focused && !item.sold {
                (ch * 0.04).max(8.0)
            } else {
                0.0
            };
            let card_rect = [cx, cy - lift, cw, ch];

            // Faint gold glow halo behind the focused card.
            if focused && !item.sold {
                let halo = ch * 0.05;
                instances.push(GpuInstance {
                    rect: [
                        card_rect[0] - halo,
                        card_rect[1] - halo,
                        card_rect[2] + halo * 2.0,
                        card_rect[3] + halo * 2.0,
                    ],
                    color: color::alpha(color::GOLD, 0.45),
                });
            }

            // Card panel — Hero variant for affordable, Default for affordable
            // but not focused, sunken-ish for sold/unaffordable.
            let variant = if item.sold {
                PanelVariant::Sunken
            } else if !can_afford {
                PanelVariant::Default
            } else if focused {
                PanelVariant::Hero
            } else {
                PanelVariant::Default
            };
            widget::push_panel(&mut instances, card_rect, variant);

            // Rarity-colored accent stripe at card top, just inside the border.
            if !item.sold {
                let bt = (ch * 0.025).clamp(1.0, 3.0);
                let stripe_h = ch * 0.04;
                instances.push(GpuInstance {
                    rect: [card_rect[0] + bt, card_rect[1] + bt, card_rect[2] - bt * 2.0, stripe_h],
                    color: rarity_accent(item.rarity),
                });
            }

            // ── Text on the card ────────────────────────────────────────
            let lifted_y = card_rect[1];
            let rarity_h = typography::size(typography::CAPTION, h);
            let rarity_y = lifted_y + ch * 0.07;
            let (rarity_text, rarity_color) = match item.rarity {
                Rarity::Common => ("Common", color::rarity(0)),
                Rarity::Uncommon => ("Uncommon", color::rarity(1)),
                Rarity::Rare => ("Rare", color::rarity(2)),
                Rarity::Legendary => ("Legendary", color::rarity(3)),
            };
            text_labels.push(TextLabel {
                rect: [cx, rarity_y, cw, rarity_h],
                text: rarity_text.to_string(),
                color: if item.sold { color::SLATE } else { rarity_color },
            });

            // Name — heading-sized gold serif.
            let name_h = typography::size(typography::HEADING, h);
            let name_y = lifted_y + ch * 0.18;
            text_labels.push(TextLabel {
                rect: [cx, name_y, cw, name_h],
                text: item.name.to_string(),
                color: if item.sold {
                    color::SLATE
                } else {
                    color::CHAMPAGNE
                },
            });

            // Description — wrapped via push_text_block so long descriptions
            // don't get crammed into a raw rect (per art-director feedback).
            let desc_y = lifted_y + ch * 0.36;
            let desc_h = ch * 0.36;
            let desc_w = cw * 0.92;
            let desc_x = cx + (cw - desc_w) * 0.5;
            let style = TextStyle {
                tier: typography::BODY,
                color: if item.sold {
                    color::SLATE
                } else {
                    color::PARCHMENT
                },
                padding: h * 0.008,
            };
            widget::push_text_block(
                &mut text_labels,
                [desc_x, desc_y, desc_w, desc_h],
                item.description,
                style,
                h,
            );

            // Price tag at the bottom of the card.
            let tag_h = ch * 0.16;
            let tag_w = cw * 0.5;
            let tag_x = cx + (cw - tag_w) * 0.5;
            let tag_y = lifted_y + ch - tag_h - ch * 0.06;
            if item.sold {
                text_labels.push(TextLabel {
                    rect: [tag_x, tag_y, tag_w, tag_h],
                    text: "SOLD".to_string(),
                    color: color::SLATE,
                });
            } else {
                widget::push_price_tag(
                    &mut instances,
                    &mut text_labels,
                    [tag_x, tag_y, tag_w, tag_h],
                    item.price,
                    can_afford,
                );
            }

            // "Can't afford" indicator for unaffordable items.
            if !item.sold && !can_afford {
                let warn_h = typography::size(typography::CAPTION, h);
                let warn_y = lifted_y + ch + ch * 0.02;
                let reason = if ctx.run.relics.is_full() {
                    "Relics full"
                } else {
                    "Not enough gold"
                };
                text_labels.push(TextLabel {
                    rect: [cx, warn_y, cw, warn_h],
                    text: reason.to_string(),
                    color: color::RUBY,
                });
            }
        }

        // ── Score panel header (SHOP) ───────────────────────────────────
        widget::push_panel(
            &mut instances,
            [sp.x, sp.y, sp.w, sp.h],
            PanelVariant::Hero,
        );
        text_labels.push(TextLabel {
            rect: [sp.x, sp.y, sp.w, sp.h],
            text: format!("SHOP  ·  Round {}  ·  Gold {}", self.came_from_round, ctx.run.gold),
            color: color::CHAMPAGNE,
        });
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
            color: color::PARCHMENT,
        });

        // ── Next Round button ───────────────────────────────────────────
        let scale = (w.min(h)) / 600.0;
        let (btn_x, btn_y, btn_w, btn_h) = Self::next_round_rect(ctx.layout);
        let next_focused = self.cursor >= total;
        let mut buttons: Vec<ButtonDef> = Vec::new();
        widget::push_button(
            &mut instances,
            &mut text_labels,
            &mut buttons,
            [btn_x, btn_y, btn_w, btn_h],
            "Next Round",
            ButtonVariant::Primary,
            if next_focused {
                ButtonState::Hover
            } else {
                ButtonState::Rest
            },
            UiAction::CommitDiscard,
        );

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
        if self.cursor >= n_items && self.cursor < total {
            let owned_idx = self.cursor - n_items;
            let rid = ctx.run.relics.active[owned_idx];
            let (rx, ry, rw, rh) = relic_badge_rect(
                &ctx.layout.relic_strip,
                w,
                ctx.run.relics.max_slots,
                owned_idx,
            );
            let pill_h = rh * 0.30;
            let pill_w = rw * 0.88;
            let pill_x = rx + (rw - pill_w) * 0.5;
            let pill_y = ry + rh - pill_h - rh * 0.04;
            widget::push_panel_colored(
                &mut instances,
                [pill_x, pill_y, pill_w, pill_h],
                color::BRASS,
                color::GOLD,
            );
            text_labels.push(TextLabel {
                rect: [pill_x, pill_y, pill_w, pill_h],
                text: format!("Sell {}g", relic_sell_price(rid)),
                color: color::OBSIDIAN,
            });
            buttons.push(ButtonDef::ui(
                (pill_x, pill_y, pill_w, pill_h),
                UiAction::Confirm,
            ));
        }

        // Pause overlay.
        self.pause_menu
            .draw(w, h, scale, &mut instances, &mut text_labels, &mut buttons);

        SceneDrawOutput {
            background: Default::default(),
            tray_instances: vec![],
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
            flame_instances: vec![],
            point_lights: vec![],
            candles: vec![],
            draw_table: false,
        }
    }
}
