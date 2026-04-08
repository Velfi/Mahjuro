//! Shop scene — between rounds; player can buy relics with gold.

use rand::seq::SliceRandom;

use crate::core::relic::{
    Rarity, RelicId, RelicState, all_relic_defs, relic_buy_price as relic_price, relic_sell_price,
};
use crate::core::zodiac::ZodiacKind;
use crate::render::theme::{ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;
use crate::ui::widget::{self, PanelVariant, TextStyle};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::pause_menu::PauseMenu;
use super::pick_blind::PickBlindScene;
use super::{
    DrawCtx, Scene, SceneBehavior, SceneDrawOutput, SceneTransition, UpdateCtx, relic_badge_rect,
    relic_row,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShopAction {
    BuyCard(usize),
    SellRelic(usize),
    BuyZodiac(usize),
    NextRound,
}

impl ShopAction {
    fn id(self) -> FocusId {
        match self {
            ShopAction::BuyCard(i) => FocusId(0x0100 + i as u32),
            ShopAction::SellRelic(i) => FocusId(0x0200 + i as u32),
            ShopAction::BuyZodiac(i) => FocusId(0x0400 + i as u32),
            ShopAction::NextRound => FocusId(0x0300),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ShopFocus {
    Card(usize),
    Owned(usize),
    Zodiac(usize),
    NextRound,
}

/// A purchasable item in the shop.
struct ShopItem {
    relic: RelicId,
    name: &'static str,
    description: &'static str,
    rarity: Rarity,
    price: u32,
    sold: bool,
}

/// A purchasable Zodiac card in the shop. Patch B finishing.
struct ZodiacItem {
    kind: ZodiacKind,
    sold: bool,
}

pub struct ShopScene {
    pub came_from_round: u32,
    items: Vec<ShopItem>,
    zodiacs: Vec<ZodiacItem>,
    tree: TreeState,
    pause_menu: PauseMenu,
    glossary: super::glossary::GlossaryOverlay,
}

/// Click id for the `?` glossary badge in the shop HUD.
const SHOP_HELP_BADGE_ID: u32 = 0x9100;

impl ShopScene {
    pub fn new(came_from_round: u32, relics: &RelicState) -> Self {
        let mut rng = rand::rng();
        let defs = all_relic_defs();
        let mut pool: Vec<&_> = defs.iter().filter(|d| !relics.has(d.id)).collect();
        pool.shuffle(&mut rng);

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

        // Patch B finishing: 3 random Zodiacs always available, 4g each.
        let mut zodiac_pool: Vec<ZodiacKind> = ZodiacKind::all().to_vec();
        zodiac_pool.shuffle(&mut rng);
        let zodiacs: Vec<ZodiacItem> = zodiac_pool
            .into_iter()
            .take(3)
            .map(|kind| ZodiacItem { kind, sold: false })
            .collect();

        let mut tree = TreeState::new();
        if !items.is_empty() {
            tree.set_focus(ShopAction::BuyCard(0).id());
        }
        Self {
            came_from_round,
            items,
            zodiacs,
            tree,
            pause_menu: PauseMenu::new(),
            glossary: super::glossary::GlossaryOverlay::new(),
        }
    }

    /// Build the flat hit-target list from current state. Used by both
    /// update() and draw() for single-source-of-truth click routing.
    fn flat_items(&self, layout: &LayoutResult, n_owned: usize, max_slots: usize) -> Vec<FlatItem<ShopAction>> {
        let mut items = Vec::with_capacity(self.items.len() + n_owned + self.zodiacs.len() + 1);
        for (i, &(cx, cy, cw, ch)) in self.card_rects(layout).iter().enumerate() {
            items.push(FlatItem::new(
                ShopAction::BuyCard(i).id(),
                [cx, cy, cw, ch],
                ShopAction::BuyCard(i),
            ));
        }
        for i in 0..n_owned {
            let (rx, ry, rw, rh) =
                relic_badge_rect(&layout.relic_strip, layout.window_w, max_slots, i);
            items.push(FlatItem::new(
                ShopAction::SellRelic(i).id(),
                [rx, ry, rw, rh],
                ShopAction::SellRelic(i),
            ));
        }
        for (i, &(zx, zy, zw, zh)) in self.zodiac_rects(layout).iter().enumerate() {
            items.push(FlatItem::new(
                ShopAction::BuyZodiac(i).id(),
                [zx, zy, zw, zh],
                ShopAction::BuyZodiac(i),
            ));
        }
        let (bx, by, bw, bh) = Self::next_round_rect(layout);
        items.push(FlatItem::new(
            ShopAction::NextRound.id(),
            [bx, by, bw, bh],
            ShopAction::NextRound,
        ));
        items
    }

    /// Zodiac strip — sits in the lower band of the hand strip, below the
    /// relic cards and above the Next Round button. Sized so it never
    /// collides with either neighbor regardless of window aspect.
    fn zodiac_rects(&self, layout: &LayoutResult) -> Vec<(f32, f32, f32, f32)> {
        let n = self.zodiacs.len();
        if n == 0 {
            return vec![];
        }
        let w = layout.window_w;
        let h = layout.window_h;
        let hs = layout.hand_strip;
        let scale = (w.min(h)) / 600.0;
        let slot_w = (180.0 * scale).max(110.0);
        let slot_h = (62.0 * scale).max(40.0);
        let gap = (16.0 * scale).max(8.0);
        let total_w = slot_w * n as f32 + gap * (n as f32 - 1.0);
        let start_x = (w - total_w) * 0.5;

        // Anchor the zodiac strip just below the relic-cards band. The cards
        // take the top 66% of the hand strip with a 6% top pad, so they end
        // around hs.y + hs.h * 0.72. We sit ~12px below that and clamp so we
        // also stay above the Next Round button.
        let cards_bottom = hs.y + hs.h * 0.72 + (12.0 * scale);
        let (_, btn_y, _, _) = Self::next_round_rect(layout);
        let max_strip_y = btn_y - slot_h - (24.0 * scale);
        let strip_y = cards_bottom.min(max_strip_y);

        (0..n)
            .map(|i| {
                let zx = start_x + i as f32 * (slot_w + gap);
                (zx, strip_y, slot_w, slot_h)
            })
            .collect()
    }

    /// Resolve the focused element's semantic kind for draw-time highlighting
    /// and description-text rendering.
    fn focused_kind(&self) -> ShopFocus {
        match self.tree.focused() {
            Some(id) => {
                if id.0 >= 0x0400 {
                    ShopFocus::Zodiac((id.0 - 0x0400) as usize)
                } else if id.0 >= 0x0300 {
                    ShopFocus::NextRound
                } else if id.0 >= 0x0200 {
                    ShopFocus::Owned((id.0 - 0x0200) as usize)
                } else {
                    ShopFocus::Card((id.0 - 0x0100) as usize)
                }
            }
            None => ShopFocus::Card(0),
        }
    }

    /// Evenly-spaced card rects within the hand strip area.
    /// Same layout used by both update() (hit-testing) and draw() (rendering).
    ///
    /// The relic cards occupy the *upper* portion of the hand strip; the
    /// lower band is reserved for the Zodiac strip so the two rows don't
    /// overlap.
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
        // Cards occupy the top ~68% of the hand strip — the bottom band is
        // for the Zodiac strip. Slightly more top padding so cards don't
        // crowd the modifier strip above.
        let pad_top = hs.h * 0.06;
        let card_h = hs.h * 0.66;
        let card_y = hs.y + pad_top;
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

}

impl SceneBehavior for ShopScene {
    fn pause_options_overlay(&self) -> Option<&super::options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    fn has_blocking_overlay(&self) -> bool {
        self.pause_menu.paused || self.glossary.open
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        // Glossary overlay (cross-input help). Toggled by `?`/F1/H or the
        // `?` badge; while open, swallows other input.
        for &cid in ctx.button_clicks {
            if cid == SHOP_HELP_BADGE_ID {
                self.glossary.toggle();
                return None;
            }
        }
        if !self.glossary.open {
            for a in ctx.actions {
                if matches!(a, UiAction::Help) {
                    self.glossary.toggle();
                    return None;
                }
            }
        } else {
            self.glossary.handle_input(ctx.actions, ctx.button_clicks);
            return None;
        }

        // Pause menu handling — drives the menu while paused and intercepts
        // the open-on-Pause shortcut. Returns immediately if either applies.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            return t;
        }

        let n_owned = ctx.run.relics.active.len();
        let max_slots = ctx.run.relics.max_slots;
        let items = self.flat_items(ctx.layout, n_owned, max_slots);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
            },
        );

        // CommitDiscard (Enter) is a shortcut that always proceeds to
        // the next round, regardless of focus. (The pause shortcut is
        // already handled by `pause_menu.handle()` above.)
        for a in ctx.actions {
            if matches!(a, UiAction::CommitDiscard) {
                return Some(Scene::PickBlind(PickBlindScene::new()));
            }
        }

        match action {
            Some(ShopAction::NextRound) => {
                return Some(Scene::PickBlind(PickBlindScene::new()));
            }
            Some(ShopAction::SellRelic(idx)) => {
                if idx < ctx.run.relics.active.len() {
                    let rid = ctx.run.relics.active[idx];
                    let refund = relic_sell_price(rid);
                    ctx.run.relics.active.remove(idx);
                    ctx.run.gold = ctx.run.gold.saturating_add(refund);
                }
                return None;
            }
            Some(ShopAction::BuyCard(idx)) => {
                if idx < self.items.len() {
                    let item = &mut self.items[idx];
                    if !item.sold && ctx.run.gold >= item.price && !ctx.run.relics.is_full() {
                        ctx.run.gold -= item.price;
                        ctx.run.relics.active.push(item.relic);
                        ctx.run.recompute_capacities();
                        item.sold = true;
                    }
                }
                return None;
            }
            Some(ShopAction::BuyZodiac(idx)) => {
                if idx < self.zodiacs.len() {
                    let item = &mut self.zodiacs[idx];
                    let price = ZodiacKind::shop_price();
                    let inventory_full = ctx.run.zodiac_inventory.is_full();
                    if !item.sold && ctx.run.gold >= price && !inventory_full {
                        ctx.run.gold -= price;
                        ctx.run.zodiac_inventory.items.push(item.kind);
                        item.sold = true;
                    }
                }
                return None;
            }
            None => {}
        }
        None
    }

    fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let sp = ctx.layout.score_panel;
        let ms = ctx.layout.modifier_strip;

        // Background — Midnight Gold deep base.
        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        }];

        let n_owned = ctx.run.relics.active.len();
        let max_slots = ctx.run.relics.max_slots;
        let focus = self.focused_kind();

        // Gold border under the focused owned relic.
        if let ShopFocus::Owned(owned_idx) = focus {
            let (rx, ry, rw, rh) =
                relic_badge_rect(&ctx.layout.relic_strip, w, max_slots, owned_idx);
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
            let focused = matches!(focus, ShopFocus::Card(j) if j == i);
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
            // Note: rasterize_label uses font_px ≈ rect.h * 0.55, so rect
            // heights need to be ~1.8× the typography tier to render at the
            // tier's intended pixel size.
            let lifted_y = card_rect[1];
            let rarity_h = typography::size(typography::BODY, h) * 1.8;
            let rarity_y = lifted_y + ch * 0.06;
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
                ..Default::default()
            });

            // Name — title-sized gold serif.
            let name_h = typography::size(typography::TITLE, h) * 1.8;
            let name_y = lifted_y + ch * 0.16;
            text_labels.push(TextLabel {
                rect: [cx, name_y, cw, name_h],
                text: item.name.to_string(),
                color: if item.sold {
                    color::SLATE
                } else {
                    color::CHAMPAGNE
                },
                ..Default::default()
            });

            // Description — wrapped via push_text_block so long descriptions
            // don't get crammed into a raw rect (per art-director feedback).
            let desc_y = lifted_y + ch * 0.36;
            let desc_h = ch * 0.36;
            let desc_w = cw * 0.92;
            let desc_x = cx + (cw - desc_w) * 0.5;
            let style = TextStyle {
                tier: typography::HEADING,
                color: if item.sold {
                    color::SLATE
                } else {
                    color::PARCHMENT
                },
                padding: h * 0.008,
                align: TextAlign::Center,
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
                    ..Default::default()
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
                let warn_h = typography::size(typography::CAPTION, h) * 1.8;
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
                    ..Default::default()
                });
            }
        }

        // ── Zodiac strip (Patch B finishing) ────────────────────────────
        //
        // 3 small cards offering Chinese Zodiac consumables for a flat price.
        // Sits just above the Next Round button so it's visible without
        // crowding the relic cards.
        let zodiac_rects = self.zodiac_rects(ctx.layout);
        for (i, &(zx, zy, zw, zh)) in zodiac_rects.iter().enumerate() {
            let z_item = &self.zodiacs[i];
            let z_focused = matches!(focus, ShopFocus::Zodiac(j) if j == i);
            let inv_full = ctx.run.zodiac_inventory.is_full();
            let can_afford_z =
                ctx.run.gold >= ZodiacKind::shop_price() && !inv_full && !z_item.sold;

            let variant = if z_item.sold {
                PanelVariant::Sunken
            } else if z_focused && can_afford_z {
                PanelVariant::Hero
            } else {
                PanelVariant::Default
            };
            widget::push_panel(&mut instances, [zx, zy, zw, zh], variant);

            // Focus halo.
            if z_focused && !z_item.sold {
                let halo = zh * 0.08;
                instances.push(GpuInstance {
                    rect: [zx - halo, zy - halo, zw + halo * 2.0, zh + halo * 2.0],
                    color: color::alpha(color::GOLD, 0.45),
                });
            }

            // Three stacked rows inside the slot: zodiac name, target yaku,
            // price tag. Font sizes are PINNED so the rasterizer can't
            // auto-shrink long yaku names (Sanshoku, Chiitoitsu) into the
            // 8px floor when the slot is narrower than ideal.
            let row_h = zh * 0.30;
            let pad_y = zh * 0.05;
            let name_font = (row_h * 0.78).clamp(15.0, 24.0);
            let yaku_font = (row_h * 0.70).clamp(13.0, 20.0);
            let price_font = (row_h * 0.78).clamp(14.0, 22.0);
            let name_color = if z_item.sold {
                color::SLATE
            } else {
                color::GOLD
            };
            text_labels.push(TextLabel {
                rect: [zx, zy + pad_y, zw, row_h],
                text: z_item.kind.name().to_string(),
                color: name_color,
                font_px: Some(name_font),
                ..Default::default()
            });
            text_labels.push(TextLabel {
                rect: [zx, zy + pad_y + row_h, zw, row_h],
                text: format!("→ {}", z_item.kind.yaku().name()),
                color: if z_item.sold {
                    color::SLATE
                } else {
                    color::PARCHMENT
                },
                font_px: Some(yaku_font),
                ..Default::default()
            });
            let tag_y = zy + pad_y + row_h * 2.0;
            if z_item.sold {
                text_labels.push(TextLabel {
                    rect: [zx, tag_y, zw, row_h],
                    text: "SOLD".to_string(),
                    color: color::SLATE,
                    font_px: Some(price_font),
                    ..Default::default()
                });
            } else {
                text_labels.push(TextLabel {
                    rect: [zx, tag_y, zw, row_h],
                    text: format!("{}g", ZodiacKind::shop_price()),
                    color: if can_afford_z { color::GOLD } else { color::RUBY },
                    font_px: Some(price_font),
                    ..Default::default()
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
            ..Default::default()
        });
        text_labels.extend(relic_labels);

        // Description of selected item in modifier strip.
        let desc_text = match focus {
            ShopFocus::Card(idx) if idx < self.items.len() => {
                let item = &self.items[idx];
                if item.sold {
                    format!("{} — SOLD", item.name)
                } else {
                    item.description.to_string()
                }
            }
            ShopFocus::Owned(idx) if idx < ctx.run.relics.active.len() => {
                let rid = ctx.run.relics.active[idx];
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
            }
            ShopFocus::Zodiac(idx) if idx < self.zodiacs.len() => {
                let z = &self.zodiacs[idx];
                if z.sold {
                    format!("{} — SOLD", z.kind.name())
                } else {
                    format!(
                        "{} ({}g) — Levels {} for the rest of the run",
                        z.kind.name(),
                        ZodiacKind::shop_price(),
                        z.kind.yaku().name()
                    )
                }
            }
            _ => "Leave shop and pick your next blind".into(),
        };
        text_labels.push(TextLabel {
            rect: [ms.x, ms.y, ms.w, ms.h],
            text: desc_text,
            color: color::PARCHMENT,
            ..Default::default()
        });

        // ── Next Round button ───────────────────────────────────────────
        let scale = (w.min(h)) / 600.0;
        let (btn_x, btn_y, btn_w, btn_h) = Self::next_round_rect(ctx.layout);
        let next_focused = matches!(focus, ShopFocus::NextRound);
        let mut buttons = Vec::new();
        // Render the button visuals — hit-target registered via flat_items below.
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
        // Drop the synthetic ButtonDef::ui that push_button added — we'll
        // re-register all hit targets through flat_items() below.
        buttons.pop();

        // Sell pill on the focused owned relic — visual only, hit-test for
        // selling routes through the badge rect itself.
        if let ShopFocus::Owned(owned_idx) = focus {
            if owned_idx < ctx.run.relics.active.len() {
                let rid = ctx.run.relics.active[owned_idx];
                let (rx, ry, rw, rh) =
                    relic_badge_rect(&ctx.layout.relic_strip, w, max_slots, owned_idx);
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
                    ..Default::default()
                });
            }
        }

        // Single hit-target list shared with update() — single source of truth.
        let items = self.flat_items(ctx.layout, n_owned, max_slots);
        self.tree.register_flat_buttons(&items, &mut buttons);

        // ── Help (`?`) badge — top-left corner ──────────────────────────
        let help_w = (38.0 * scale).max(24.0);
        let help_h = help_w;
        let help_x = (12.0 * scale).max(8.0);
        let help_y = sp.y + (sp.h - help_h) * 0.5;
        instances.push(GpuInstance {
            rect: [help_x, help_y, help_w, help_h],
            color: color::alpha(color::INDIGO, 0.92),
        });
        let rim = (1.5 * scale).max(1.0);
        instances.push(GpuInstance {
            rect: [help_x, help_y, help_w, rim],
            color: color::GOLD,
        });
        instances.push(GpuInstance {
            rect: [help_x, help_y + help_h - rim, help_w, rim],
            color: color::GOLD,
        });
        instances.push(GpuInstance {
            rect: [help_x, help_y, rim, help_h],
            color: color::GOLD,
        });
        instances.push(GpuInstance {
            rect: [help_x + help_w - rim, help_y, rim, help_h],
            color: color::GOLD,
        });
        text_labels.push(TextLabel {
            rect: [help_x, help_y, help_w, help_h],
            text: "?".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });
        buttons.push(super::ButtonDef::scene(
            (help_x, help_y, help_w, help_h),
            SHOP_HELP_BADGE_ID,
        ));

        // Pause overlay.
        self.pause_menu
            .draw(w, h, scale, &mut instances, &mut text_labels, &mut buttons);

        // Glossary overlay (drawn last so it covers everything).
        self.glossary
            .draw(w, h, &mut instances, &mut text_labels, &mut buttons);
        if self.glossary.open {
            buttons.push(super::ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

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
            relic_placements: vec![],
            draw_table: false,
            wind_gusts: Vec::new(),
        }
    }
}
