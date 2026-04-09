//! Shop scene — between rounds; player can buy relics with gold.
//!
//! The shop renders as a 3D curio shop: a wooden curio cabinet (back wall
//! shadow box) holds the for-sale relics in inset compartments on its left
//! half and the for-sale zodiac/talisman ribbons pinned in a row on its
//! right half. In the foreground, two brass dishes (owned-relic dish + coin
//! dish) sit on the counter, with the player's owned consumables fanned out
//! beside them like ribbons in hand.
//!
//! Hovering an object turns on a literal point-light spotlight on it (the
//! same lighting mechanism gameplay uses for tile highlights). A 2D tooltip
//! panel anchored above the projected screen rect of the hovered object
//! shows its name + a Buy/Sell call-to-action.

use rand::RngExt;
use rand::seq::SliceRandom;

use crate::core::consumable::Consumable;
use crate::core::relic::{
    Rarity, RelicId, RelicState, all_relic_defs, relic_buy_price as relic_price, relic_sell_price,
};
use crate::core::talisman::TalismanKind;
use crate::core::zodiac::ZodiacKind;
use crate::render::curio_cabinet_mesh::{NICHE_COLS, NICHE_ROWS, niche_centers_local};
use crate::render::draw_cmd::{
    CameraParams, CoinPlacement, CurioCabinetPlacement, DishExplicit, PlaquePlacement,
    RelicPlacement, TalismanPlacement, UiFrame, ZodiacRibbonPlacement,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, ShopHit, TextAlign, TextLabel};
use crate::ui::focus_nav::{FocusDir, focus_target_at_cursor, pick_neighbor, push_focus_ring};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::widget::{self, PanelVariant, TextStyle};

use super::pause_menu::PauseMenu;
use super::pick_blind::PickBlindScene;
use super::{
    BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneDrawOutput, SceneTransition,
    UpdateCtx,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShopAction {
    BuyCard(usize),
    SellRelic(usize),
    BuyConsumable(usize),
    /// Sell the consumable at this index in `run.consumables.items`.
    SellConsumable(usize),
}

/// Every shop element a controller / keyboard player can navigate to.
/// Mirrors the [`ShopHit`] flat-index scheme so the same action-dispatch
/// path that handles mouse clicks can also handle Confirm presses, plus
/// a synthetic `NextRound` variant for the 2D button at the bottom of
/// the screen (which has no 3D pick equivalent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShopFocus {
    /// Index into the renderer's flat relic list — for-sale niches first,
    /// then owned-dish relics. Matches `ShopHit::Relic(i)`.
    Relic(usize),
    /// Index into the for-sale-zodiacs-then-owned-zodiacs flat list.
    /// Matches `ShopHit::Ribbon(i)`.
    Ribbon(usize),
    /// Index into the for-sale-talismans-then-owned-talismans flat list.
    /// Matches `ShopHit::Talisman(i)`.
    Talisman(usize),
    /// Pick id of a foreground dish — relic dish, coin dish, or journal
    /// book. Matches `ShopHit::Dish(id)`.
    Dish(u32),
    /// The 2D "Next Round" button at the bottom of the screen.
    NextRound,
}

impl ShopFocus {
    fn from_hit(h: ShopHit) -> Self {
        match h {
            ShopHit::Relic(i) => Self::Relic(i),
            ShopHit::Ribbon(i) => Self::Ribbon(i),
            ShopHit::Talisman(i) => Self::Talisman(i),
            ShopHit::Dish(id) => Self::Dish(id),
        }
    }
    /// The equivalent [`ShopHit`] for the variants that have one — i.e.
    /// everything except `NextRound`. Used to feed the focus state into
    /// the spotlight + tooltip + click-dispatch paths that already key
    /// off `ShopHit`.
    fn to_hit(self) -> Option<ShopHit> {
        match self {
            Self::Relic(i) => Some(ShopHit::Relic(i)),
            Self::Ribbon(i) => Some(ShopHit::Ribbon(i)),
            Self::Talisman(i) => Some(ShopHit::Talisman(i)),
            Self::Dish(id) => Some(ShopHit::Dish(id)),
            Self::NextRound => None,
        }
    }
}

/// Map a `ShopHit` (from a click *or* a focus Confirm) to the
/// corresponding `ShopAction`. Pure lookup against the current shop
/// stock + run state — no mutation. Dishes are info-only and return
/// `None`. Returns `None` for the journal book too; the caller handles
/// the journal toggle separately because it switches scene state
/// (`self.journal.open`) rather than running an action.
fn shop_action_for_hit(
    hit: ShopHit,
    items: &[ShopItem],
    consumable_items: &[ConsumableShopItem],
    run: &crate::game::run::RunState,
) -> Option<ShopAction> {
    let n_for_sale = items.len();
    match hit {
        ShopHit::Relic(i) => {
            if i < n_for_sale {
                Some(ShopAction::BuyCard(i))
            } else {
                Some(ShopAction::SellRelic(i - n_for_sale))
            }
        }
        ShopHit::Ribbon(i) => {
            // Ribbon hits index into the for-sale-zodiacs-then-owned-
            // zodiacs flat list. Walk consumable_items in order, take
            // the i-th zodiac. The for-sale section emits Buy; the
            // owned-fan section emits Sell.
            let zodiac_for_sale: Vec<usize> = consumable_items
                .iter()
                .enumerate()
                .filter_map(|(idx, c)| matches!(c.consumable, Consumable::Zodiac(_)).then_some(idx))
                .collect();
            if i < zodiac_for_sale.len() {
                Some(ShopAction::BuyConsumable(zodiac_for_sale[i]))
            } else {
                let oi = i - zodiac_for_sale.len();
                let mut count = 0usize;
                let mut inv_idx = None;
                for (idx, c) in run.consumables.items.iter().enumerate() {
                    if matches!(c, Consumable::Zodiac(_)) {
                        if count == oi {
                            inv_idx = Some(idx);
                            break;
                        }
                        count += 1;
                    }
                }
                inv_idx.map(ShopAction::SellConsumable)
            }
        }
        ShopHit::Talisman(i) => {
            let talisman_for_sale: Vec<usize> = consumable_items
                .iter()
                .enumerate()
                .filter_map(|(idx, c)| {
                    matches!(c.consumable, Consumable::Talisman(_)).then_some(idx)
                })
                .collect();
            if i < talisman_for_sale.len() {
                Some(ShopAction::BuyConsumable(talisman_for_sale[i]))
            } else {
                let oi = i - talisman_for_sale.len();
                let mut count = 0usize;
                let mut inv_idx = None;
                for (idx, c) in run.consumables.items.iter().enumerate() {
                    if matches!(c, Consumable::Talisman(_)) {
                        if count == oi {
                            inv_idx = Some(idx);
                            break;
                        }
                        count += 1;
                    }
                }
                inv_idx.map(ShopAction::SellConsumable)
            }
        }
        ShopHit::Dish(_) => None,
    }
}

/// Apply a `ShopAction` to the shop's stock + the player's run state.
/// Pulled out of the inline click-dispatch code so the focus model's
/// `Confirm` path can fire the same action handlers without
/// duplication.
fn apply_shop_action(
    action: ShopAction,
    items: &mut Vec<ShopItem>,
    consumable_items: &mut Vec<ConsumableShopItem>,
    run: &mut crate::game::run::RunState,
) {
    match action {
        ShopAction::BuyCard(idx) => {
            if idx < items.len() {
                let item = &items[idx];
                if !item.sold && run.gold >= item.price && !run.relics.is_full() {
                    let price = item.price;
                    let relic = item.relic;
                    run.gold -= price;
                    run.relics.active.push(relic);
                    run.recompute_capacities();
                    items.remove(idx);
                }
            }
        }
        ShopAction::SellRelic(idx) => {
            if idx < run.relics.active.len() {
                let rid = run.relics.active[idx];
                let refund = relic_sell_price(rid);
                run.relics.active.remove(idx);
                run.gold = run.gold.saturating_add(refund);
            }
        }
        ShopAction::BuyConsumable(idx) => {
            if idx < consumable_items.len() {
                let item = &consumable_items[idx];
                let price = item.price();
                if !item.sold && run.gold >= price && !run.consumables.is_full() {
                    let consumable = item.consumable;
                    run.gold -= price;
                    run.consumables.items.push(consumable);
                    consumable_items.remove(idx);
                }
            }
        }
        ShopAction::SellConsumable(idx) => {
            if idx < run.consumables.items.len() {
                let c = run.consumables.items[idx];
                let refund = consumable_sell_price(c);
                run.consumables.items.remove(idx);
                run.gold = run.gold.saturating_add(refund);
            }
        }
    }
}

/// Refund when selling a consumable — half buy price, minimum 1 gold,
/// matching the relic sell formula.
fn consumable_sell_price(c: Consumable) -> u32 {
    let buy = match c {
        Consumable::Zodiac(_) => ZodiacKind::shop_price(),
        Consumable::Talisman(_) => TalismanKind::shop_price(),
    };
    (buy / 2).max(1)
}

/// A purchasable relic in the shop.
struct ShopItem {
    relic: RelicId,
    name: &'static str,
    description: &'static str,
    rarity: Rarity,
    price: u32,
    sold: bool,
}

/// A purchasable consumable in the shop — either a Zodiac or a Talisman.
/// Both share the player's [`crate::core::consumable::ConsumableInventory`]
/// slots.
struct ConsumableShopItem {
    consumable: Consumable,
    sold: bool,
}

impl ConsumableShopItem {
    fn price(&self) -> u32 {
        match self.consumable {
            Consumable::Zodiac(_) => ZodiacKind::shop_price(),
            Consumable::Talisman(_) => TalismanKind::shop_price(),
        }
    }
    fn name(&self) -> String {
        self.consumable.name()
    }
    fn description(&self) -> String {
        match self.consumable {
            Consumable::Zodiac(z) => format!(
                "Levels {} for the rest of the run (+0.5 mult, +20 chips per level).",
                z.yaku().name()
            ),
            Consumable::Talisman(t) => t.description().to_string(),
        }
    }
}

pub struct ShopScene {
    pub came_from_round: u32,
    items: Vec<ShopItem>,
    consumable_items: Vec<ConsumableShopItem>,
    pause_menu: PauseMenu,
    glossary: super::glossary::GlossaryOverlay,
    journal: super::journal::JournalOverlay,
    /// Currently focused shop element. `None` until the player presses a
    /// directional input or moves the cursor over a shop object.
    focus: Option<ShopFocus>,
    /// Focus rect graph captured at the end of the previous `draw_frame`,
    /// consumed by `update()` for cursor hit-tests and spatial navigation.
    /// One frame stale — same pattern as `projected_relic_rects` and the
    /// gameplay scene's identical mechanism. Wrapped in a `RefCell` because
    /// `draw_frame` takes `&self` but needs to update this stash.
    last_focus_rects: std::cell::RefCell<Vec<(ShopFocus, [f32; 4])>>,
}

/// Click id for the `?` glossary badge in the shop HUD.
const SHOP_HELP_BADGE_ID: u32 = 0x9100;
/// Click id for the catch-all 3D-hit dispatcher. When clicked, the shop's
/// update() routes the click based on `UpdateCtx::picked_shop_object`.
const SHOP_3D_HIT_ID: u32 = 0x9200;
/// Click id for the Next Round 2D button.
const SHOP_NEXT_ROUND_ID: u32 = 0x9300;
/// Pick id for the foreground relic dish.
const PICK_RELIC_DISH: u32 = 1;
/// Pick id for the coin dish.
const PICK_COIN_DISH: u32 = 2;
/// Pick id for the Yaku Journal book on the shop counter. Reuses the
/// existing `DishExplicit` + `ShopHit::Dish(u32)` pick path so the shop
/// can offer the journal without renderer changes — the silhouette is
/// dish-shaped until proper book art lands, but the click target is
/// what matters here.
const PICK_JOURNAL_BOOK: u32 = 3;

impl ShopScene {
    pub fn new(came_from_round: u32, relics: &RelicState) -> Self {
        let mut rng = rand::rng();

        // Randomize stock independently per shop visit. The cabinet has
        // NICHE_COLS*NICHE_ROWS relic compartments and ~MAX_RIBBONS ribbon
        // anchors; we pick how many of each kind to actually offer this
        // visit so the shop's mix varies (sometimes only relics, sometimes
        // only consumables, sometimes a small handful of each).
        const MAX_RIBBONS: usize = 3;
        let max_relics = NICHE_COLS * NICHE_ROWS;

        // For each category, roll 0..=cap with a slight bias toward
        // having *some* of each so the shop usually feels stocked.
        // Zodiacs and talismans both floor at 1 so the consumable wall is
        // never empty — players were complaining the shop "had no zodiacs"
        // because the previous 0..=MAX roll left it empty too often.
        let mut n_relics = rng.random_range(0..=max_relics);
        let mut n_zodiacs = rng.random_range(1..=MAX_RIBBONS);
        let mut n_talismans = rng.random_range(1..=MAX_RIBBONS);
        // Cap total ribbons across both consumable types so we don't
        // overflow the cabinet's right half.
        if n_zodiacs + n_talismans > MAX_RIBBONS {
            // Trim from whichever is larger.
            while n_zodiacs + n_talismans > MAX_RIBBONS {
                if n_zodiacs >= n_talismans {
                    n_zodiacs -= 1;
                } else {
                    n_talismans -= 1;
                }
            }
        }
        // Floor: shop must always offer at least 2 items total. Bump the
        // smallest category until we have ≥ 2, respecting per-category caps
        // and the combined ribbon cap.
        while n_relics + n_zodiacs + n_talismans < 2 {
            // Pick a category that still has room.
            let relics_room = n_relics < max_relics;
            let ribbons_room = n_zodiacs + n_talismans < MAX_RIBBONS;
            let zodiacs_room = ribbons_room && n_zodiacs < MAX_RIBBONS;
            let talismans_room = ribbons_room && n_talismans < MAX_RIBBONS;
            let mut choices: Vec<u8> = Vec::with_capacity(3);
            if relics_room {
                choices.push(0);
            }
            if zodiacs_room {
                choices.push(1);
            }
            if talismans_room {
                choices.push(2);
            }
            if choices.is_empty() {
                break;
            }
            match choices[rng.random_range(0..choices.len())] {
                0 => n_relics += 1,
                1 => n_zodiacs += 1,
                _ => n_talismans += 1,
            }
        }

        // Build the relic stock from the player's unowned-relic pool.
        let defs = all_relic_defs();
        let mut relic_pool: Vec<&_> = defs.iter().filter(|d| !relics.has(d.id)).collect();
        relic_pool.shuffle(&mut rng);
        let items: Vec<ShopItem> = relic_pool
            .into_iter()
            .take(n_relics)
            .map(|d| ShopItem {
                relic: d.id,
                name: d.name,
                description: d.description,
                rarity: d.rarity,
                price: relic_price(d.id),
                sold: false,
            })
            .collect();

        // Build the consumable stock as separate zodiac + talisman picks
        // so the rolled counts are honored exactly.
        let mut zodiac_pool: Vec<ZodiacKind> = ZodiacKind::all().iter().copied().collect();
        zodiac_pool.shuffle(&mut rng);
        let mut talisman_pool: Vec<TalismanKind> = TalismanKind::all().iter().copied().collect();
        talisman_pool.shuffle(&mut rng);
        let mut consumable_items: Vec<ConsumableShopItem> = zodiac_pool
            .into_iter()
            .take(n_zodiacs)
            .map(|z| ConsumableShopItem {
                consumable: Consumable::Zodiac(z),
                sold: false,
            })
            .chain(
                talisman_pool
                    .into_iter()
                    .take(n_talismans)
                    .map(|t| ConsumableShopItem {
                        consumable: Consumable::Talisman(t),
                        sold: false,
                    }),
            )
            .collect();
        // Shuffle so zodiacs and talismans interleave on the wall instead
        // of always grouping zodiacs first.
        consumable_items.shuffle(&mut rng);

        Self {
            came_from_round,
            items,
            consumable_items,
            pause_menu: PauseMenu::new(),
            glossary: super::glossary::GlossaryOverlay::new(),
            journal: super::journal::JournalOverlay::new(),
            focus: None,
            last_focus_rects: std::cell::RefCell::new(Vec::new()),
        }
    }
}

/// Spatial layout of the 3D shop scene, computed once per frame from the
/// window size. All `pixel_*` fields use the renderer's `(pixel_x, pixel_y)`
/// convention; the renderer maps them onto the table plane via its
/// `pixel_to_world` helper.
#[derive(Clone, Copy)]
struct ShopLayout {
    // ── Camera ──
    camera: CameraParams,
    // ── Curio cabinet (back wall) ──
    cabinet_pixel_x: f32,
    cabinet_pixel_y: f32,
    cabinet_world_y: f32,
    cabinet_extents: [f32; 3],
    // ── For-sale relic niches (in pixel space, world_y = base height) ──
    niche_centers_px: [(f32, f32, f32); NICHE_COLS * NICHE_ROWS],
    niche_count: usize,
    // ── For-sale ribbon anchors ──
    ribbon_anchors_px: [(f32, f32, f32); 8],
    ribbon_count: usize,
    ribbon_length: f32,
    ribbon_width: f32,
    // ── Foreground dishes ──
    relic_dish_center_px: (f32, f32, f32),
    relic_dish_extents: [f32; 3],
    coin_dish_center_px: (f32, f32, f32),
    coin_dish_extents: [f32; 3],
    // ── Owned-relic positions inside the relic dish ──
    owned_relic_count: usize,
    // ── Owned consumable row (flat tray on the inventory shelf) ──
    consumable_row_center_px: (f32, f32, f32),
    consumable_row_extents: [f32; 3],
    consumable_count: usize,
    consumable_length: f32,
    consumable_width: f32,
}

impl ShopLayout {
    fn build(
        layout: &crate::ui::layout::LayoutResult,
        n_for_sale: usize,
        n_for_sale_ribbons: usize,
        n_owned_relics: usize,
        n_fan: usize,
    ) -> Self {
        let w = layout.window_w;
        let h = layout.window_h;
        // ── Camera: shop perspective ─────────────────────────────────────
        // Pulled in close with a wide FOV so the scene fills the screen
        // on small/handheld displays — wasted black space around the
        // cabinet was the dominant readability problem at low resolution.
        let camera = CameraParams {
            eye: [0.0, h * 0.45, h * 0.95],
            target: [0.0, h * 0.30, -h * 0.20],
            up: [0.0, 1.0, 0.0],
            fovy_deg: 60.0,
        };

        // ── Cabinet (back wall) ──────────────────────────────────────────
        // Sized to fill most of the screen width and the upper two-thirds
        // of the screen height. The previous w*0.60 × h*0.55 cabinet only
        // covered ~40% of the viewport once perspective foreshortening
        // was applied, leaving handheld players staring at black borders.
        // Cabinet z-depth in real units — a curio cabinet is typically
        // ~200-300mm front-to-back. Width and height stay layout-relative
        // because the cabinet must fill most of the screen at any
        // resolution; only the depth is a true physical measurement.
        // Pushed back in depth so the cabinet sits well behind the
        // inventory dish band. In the renderer's coord system the
        // *second* component of `center_pos` is the world-Z anchor (depth)
        // and the *third* is world-Y (vertical) — see `pixel_to_world` in
        // wgpu_renderer.rs. Smaller `cabinet_pixel_y` = further from the
        // camera. We slot the cabinet further upstage and bump its size
        // back up to compensate for the perspective shrink.
        let cabinet_extents = [w * 0.92, h * 0.78, layout.mm(220.0)];
        let cabinet_pixel_x = w * 0.5;
        let cabinet_pixel_y = -h * 0.10;
        let cabinet_world_y = h * 0.42;
        // Pixel-y of the cabinet's *front face* (toward camera). Anything
        // we want to display "in" the cabinet should sit at or in front
        // of this, otherwise it gets hidden behind the cabinet's wood.
        let cabinet_front_py = cabinet_pixel_y + cabinet_extents[2] * 0.5;

        // For-sale relic niches: transform niche local coords by cabinet
        // center + extents. Pull the relics forward so they sit just
        // *inside* the front of their compartment, not buried at the back.
        let mut niche_centers_px: [(f32, f32, f32); NICHE_COLS * NICHE_ROWS] =
            [(0.0, 0.0, 0.0); NICHE_COLS * NICHE_ROWS];
        let niche_locals = niche_centers_local(NICHE_COLS * NICHE_ROWS);
        let n_niches = n_for_sale.min(niche_locals.len());
        for (i, lpt) in niche_locals.iter().take(n_niches).enumerate() {
            let px = cabinet_pixel_x + lpt[0] * cabinet_extents[0];
            // Sit relics ~30% inside the cabinet from the front face — far
            // enough back to read as "in" the compartment, far enough
            // forward to clear the front frame and catch light cleanly.
            let py = cabinet_front_py - cabinet_extents[2] * 0.30;
            // World-y of the relic — at the center height of its compartment.
            let wy = cabinet_world_y + lpt[1] * cabinet_extents[1] - cabinet_extents[1] * 0.04;
            niche_centers_px[i] = (px, py, wy);
        }

        // For-sale ribbon anchors: spread across the (narrower) right side
        // of the cabinet, pinned at the *front face* so they hang visibly
        // in front of the back panel. The cabinet is asymmetric — the
        // central divider sits at local-x = DIVIDER_X (≈+0.10), so the
        // ribbon area runs from just-past-the-divider out to the inner
        // edge of the right frame strip.
        let mut ribbon_anchors_px: [(f32, f32, f32); 8] = [(0.0, 0.0, 0.0); 8];
        let n_ribbons = n_for_sale_ribbons.min(8);
        if n_ribbons > 0 {
            // Inner-right-area bounds in local-x (matches curio_cabinet_mesh).
            let right_lx0 = crate::render::curio_cabinet_mesh::DIVIDER_X + 0.0125;
            let right_lx1 = 0.5 - 0.04; // FRAME
            let right_inner_w = right_lx1 - right_lx0;
            // Tightened spread (0.25..0.78 of the inner area) so 3 ribbons
            // read as a deliberate grouping rather than scattered laundry.
            let right_x0 =
                cabinet_pixel_x + (right_lx0 + right_inner_w * 0.25) * cabinet_extents[0];
            let right_x1 =
                cabinet_pixel_x + (right_lx0 + right_inner_w * 0.78) * cabinet_extents[0];
            let avail = right_x1 - right_x0;
            let step = if n_ribbons > 1 {
                avail / (n_ribbons as f32 - 1.0)
            } else {
                0.0
            };
            // Pin near the very top of the cabinet's interior, just below
            // the front frame, so the hanging ribbons fill the upper third
            // of the cabinet where the camera frames them cleanly.
            let pin_world_y = cabinet_world_y + cabinet_extents[1] * 0.46;
            // Pin slightly *in front of* the cabinet's front face so the
            // ribbon hangs toward the player and doesn't z-fight the wood.
            // Scaled with cabinet depth so it stays consistent at any size.
            let pin_pixel_y = cabinet_front_py + cabinet_extents[2] * 0.05;
            for i in 0..n_ribbons {
                let x = if n_ribbons == 1 {
                    (right_x0 + right_x1) * 0.5
                } else {
                    right_x0 + step * i as f32
                };
                ribbon_anchors_px[i] = (x, pin_pixel_y, pin_world_y);
            }
        }
        // Hanging zodiac/talisman ribbons — sized layout-relative (not mm)
        // so they hold their visual weight inside the (also layout-relative)
        // cabinet at every resolution. Earlier we used mm-based dimensions,
        // which made ribbons read as postage stamps lost on a wide back wall
        // at high-DPI. Talisman extents derive from `ribbon_width`, so they
        // scale automatically.
        //
        // Width drives the rendered ribbon size now that the renderer fits
        // textured ribbons to the silk texture's natural 2:3 aspect (length =
        // width × 1.5). `ribbon_length` stays as the upper bound the
        // aspect-fit clamps against, so adjusting just `ribbon_width` is
        // enough to resize the wall ribbons proportionally.
        let ribbon_width = h * 0.085;
        let ribbon_length = ribbon_width * 1.5;

        // ── Inventory shelf (bottom band) ────────────────────────────────
        // The bottom ~25% of the screen is the player's inventory shelf:
        // a single tray broken into three sub-zones — owned relics on the
        // left (widest), owned consumables center-right, coin dish on the
        // far right. Symmetry of language: relics and consumables share
        // the same "laid flat in a tray" treatment so they parse as "your
        // collection" rather than two unrelated systems.
        // Inventory shelf. With the wide-FOV close camera, anything below
        // ~h*0.74 pixel-y projects into the dark bottom corners outside
        // the cabinet's lit area. The shelf is lifted to h*0.70 and all
        // tray x-positions are tightened toward the screen center so the
        // player's possessions read as one cohesive band rather than
        // four objects flung to the corners.
        let shelf_y = h * 0.70;
        // All three inventory trays share a ~12mm brass rim — small
        // decorative dishes that hold the player's items. Footprint
        // (width/depth) stays layout-relative so the shelf reads as a
        // wide band across the bottom of the screen at any resolution.
        let dish_rim = layout.mm(12.0);
        // Relic tray spans 16%–44% w → center at 30% w, half-width 14% w.
        let relic_dish_center_px = (w * 0.30, shelf_y, 0.0);
        let relic_dish_extents = [w * 0.14, dish_rim, h * 0.10];
        // Consumable tray spans 48%–68% w → center at 58% w, half-width 10% w.
        let consumable_row_center_px = (w * 0.58, shelf_y, 0.0);
        let consumable_row_extents = [w * 0.10, dish_rim, h * 0.10];
        // Coin dish — sits just right of the consumable tray as the
        // rightmost item on the shelf. 71.5%–80.5% w.
        let coin_dish_center_px = (w * 0.76, shelf_y, 0.0);
        let coin_dish_extents = [w * 0.045, dish_rim, h * 0.07];

        // ── Owned consumables: flat row in the consumable tray ───────────
        // Laid flat (face-up) like the relic tray, no radial fan. The
        // ribbon length must fit inside the tray's z-extent (h*0.10) once
        // rotated -90° about X, otherwise the back end of the ribbon (and
        // the talisman center, which sits at `awy - length*0.4`) sinks
        // below the tray surface and reads as "items submerged in wood."
        // Owned-consumable face plates laid flat in the tray. Width drives
        // the visible size; the renderer enforces a 2:3 aspect when a
        // zodiac silk texture is bound, so we set length = width × 1.5
        // here to match (talisman tablets reuse `consumable_width` for
        // their footprint, so this also keeps tablet proportions sensible).
        let consumable_width = layout.mm(9.0);
        let consumable_length = consumable_width * 1.5;

        Self {
            camera,
            cabinet_pixel_x,
            cabinet_pixel_y,
            cabinet_world_y,
            cabinet_extents,
            niche_centers_px,
            niche_count: n_niches,
            ribbon_anchors_px,
            ribbon_count: n_ribbons,
            ribbon_length,
            ribbon_width,
            relic_dish_center_px,
            relic_dish_extents,
            coin_dish_center_px,
            coin_dish_extents,
            owned_relic_count: n_owned_relics,
            consumable_row_center_px,
            consumable_row_extents,
            consumable_count: n_fan,
            consumable_length,
            consumable_width,
        }
    }

    /// Center pixel coords + base world_y of an *owned* relic sitting in the
    /// foreground relic dish. Lays them out in a single row across the dish.
    fn owned_relic_pos(&self, idx: usize) -> (f32, f32, f32) {
        let n = self.owned_relic_count.max(1) as f32;
        let dish_w = self.relic_dish_extents[0] * 0.85;
        let start_x = self.relic_dish_center_px.0 - dish_w * 0.5 + (dish_w / n) * 0.5;
        let px = start_x + (dish_w / n) * idx as f32;
        let py = self.relic_dish_center_px.1;
        let wy = self.relic_dish_extents[1] * 0.5 + 4.0;
        (px, py, wy)
    }

    /// Center pixel coords + base world_y of the i-th owned consumable
    /// laid flat in the consumable tray. Single row, evenly spaced across
    /// the tray width — capped at 8 items by the caller.
    fn consumable_pos(&self, idx: usize) -> (f32, f32, f32) {
        let n = self.consumable_count.max(1) as f32;
        let row_w = self.consumable_row_extents[0] * 0.85;
        let start_x = self.consumable_row_center_px.0 - row_w * 0.5 + (row_w / n) * 0.5;
        let px = start_x + (row_w / n) * idx as f32;
        let py = self.consumable_row_center_px.1;
        // Lift the anchor well above the tray top so that talismans
        // (centered at `awy - length*0.4`) and the back end of laid-flat
        // ribbons stay above the wood instead of clipping into it.
        let wy = self.consumable_row_extents[1] * 0.5 + self.consumable_length * 0.5 + 6.0;
        (px, py, wy)
    }
}

/// Color the i-th relic rarity uses for its 3D cuboid.
fn rarity_color(rarity: Rarity) -> [f32; 4] {
    let tier = match rarity {
        Rarity::Common => 0,
        Rarity::Uncommon => 1,
        Rarity::Rare => 2,
        Rarity::Legendary => 3,
    };
    color::rarity(tier)
}

/// Deterministic per-relic half-extents derived from the RelicId discriminant
/// so each cuboid reads as a distinct object. Sized to fit comfortably inside
/// a curio cabinet niche.
fn relic_half_extents(id: RelicId, base: f32) -> [f32; 3] {
    let seed = (id as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
    let r0 = ((seed >> 8) & 0xFF) as f32 / 255.0;
    let r1 = ((seed >> 16) & 0xFF) as f32 / 255.0;
    let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
    [
        base * (0.65 + r0 * 0.45),
        base * (0.65 + r1 * 0.45),
        base * (0.55 + r2 * 0.40),
    ]
}

/// Color the i-th consumable's ribbon. Zodiacs use a palette indexed by
/// kind discriminant; talismans get a per-variant tint that mirrors the
/// gemstone the talisman is named after, so the player can tell at a
/// glance which enhancement a tablet will stamp onto the hand.
fn consumable_color(c: Consumable) -> [f32; 4] {
    match c {
        Consumable::Zodiac(z) => {
            // Cycle through warm shop palette so each zodiac reads distinct.
            let palette = [
                [0.96, 0.62, 0.42, 1.0], // peach
                [0.95, 0.78, 0.32, 1.0], // gold
                [0.78, 0.42, 0.34, 1.0], // brick
                [0.50, 0.78, 0.55, 1.0], // jade
                [0.55, 0.62, 0.92, 1.0], // sky
                [0.85, 0.55, 0.85, 1.0], // mauve
                [0.92, 0.46, 0.62, 1.0], // rose
                [0.88, 0.86, 0.55, 1.0], // straw
                [0.45, 0.72, 0.78, 1.0], // teal
                [0.95, 0.50, 0.30, 1.0], // ember
                [0.62, 0.85, 0.42, 1.0], // moss
                [0.78, 0.66, 0.92, 1.0], // lavender
            ];
            palette[(z as usize) % palette.len()]
        }
        Consumable::Talisman(t) => match t {
            TalismanKind::Jade => [0.42, 0.82, 0.55, 1.0], // jade green
            TalismanKind::Pearl => [0.94, 0.95, 0.98, 1.0], // pearl white
            TalismanKind::Gilded => [0.96, 0.78, 0.30, 1.0], // gold
            TalismanKind::Polychrome => [0.82, 0.55, 0.95, 1.0], // iridescent violet
        },
    }
}

/// Cheap deterministic PRNG for laying out coin piles. Same xorshift the
/// renderer uses internally.
struct CoinRand(u32);
impl CoinRand {
    fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32) / u32::MAX as f32
    }
}

/// Lay out a pile of `gold` coins inside the coin dish. Coins pack into
/// hexagonal layers; once a layer fills, the pile starts a new layer above.
fn coin_pile_layout(
    gold: u32,
    dish_center_px: (f32, f32, f32),
    dish_extents: [f32; 3],
    seed: u32,
) -> Vec<CoinPlacement> {
    let n = (gold as usize).min(crate::render::wgpu_renderer::MAX_COIN_SLOTS);
    if n == 0 {
        return Vec::new();
    }
    let coin_radius = 9.0_f32;
    let coin_thickness = 3.5_f32;
    // Available footprint: a slightly inset square inside the dish bounds.
    // (Hex packing inside a circle is fiddly; an inset square is fine.)
    let inset_x = dish_extents[0] * 0.42;
    let inset_z = dish_extents[2] * 0.42;
    // Hex grid spacing (face-to-face).
    let dx = coin_radius * 2.05;
    let dz = coin_radius * 1.78;
    // Per-layer columns/rows.
    let cols = ((inset_x * 2.0) / dx).floor() as i32;
    let rows = ((inset_z * 2.0) / dz).floor() as i32;
    let cols = cols.max(1);
    let rows = rows.max(1);
    let per_layer = (cols * rows) as usize;
    let mut out = Vec::with_capacity(n);
    let mut rng = CoinRand::new(seed);
    let dish_top_y = dish_extents[1] + 2.0;
    for i in 0..n {
        let layer = i / per_layer.max(1);
        let in_layer = i % per_layer.max(1);
        let r = (in_layer as i32) / cols;
        let c = (in_layer as i32) % cols;
        // Hex offset every other row.
        let row_offset = if r % 2 == 0 { 0.0 } else { dx * 0.5 };
        let lx = -inset_x + dx * 0.5 + c as f32 * dx + row_offset;
        let lz = -inset_z + dz * 0.5 + r as f32 * dz;
        // Tiny per-coin jitter so the pile doesn't read as a perfect grid.
        let jitter_x = (rng.next() - 0.5) * 1.5;
        let jitter_z = (rng.next() - 0.5) * 1.5;
        let jitter_y = (rng.next() - 0.5) * 0.4;
        let rot_y = (rng.next() - 0.5) * std::f32::consts::TAU;
        let world_y = dish_top_y + layer as f32 * coin_thickness + jitter_y;
        out.push(CoinPlacement {
            world_pos: [
                dish_center_px.0 + lx + jitter_x,
                dish_center_px.1 + lz + jitter_z,
                world_y,
            ],
            rotation_y: rot_y,
            radius: coin_radius,
            thickness: coin_thickness,
            color: [1.00, 0.78, 0.30, 1.0],
        });
    }
    out
}

impl SceneBehavior for ShopScene {
    fn pause_options_overlay(&self) -> Option<&super::options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    fn has_blocking_overlay(&self) -> bool {
        self.pause_menu.paused || self.glossary.open || self.journal.open
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        // Glossary overlay (cross-input help).
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

        // Yaku Journal overlay — opened by clicking the Journal book on
        // the counter (routed via the 3D-hit dispatcher below as
        // `ShopHit::Dish(PICK_JOURNAL_BOOK)`).
        if self.journal.open {
            self.journal.handle_input(ctx.actions, ctx.button_clicks);
            return None;
        }

        // Pause menu handling.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            // Drain a one-shot glossary request from the pause menu.
            if self.pause_menu.take_glossary_request() {
                self.glossary.toggle();
            }
            return t;
        }

        // ── Focus model: cursor sync, spatial navigation, Confirm ──────
        //
        // The shop's focus_rect_graph is rebuilt every draw frame from
        // the projected screen rects of every focusable shop element
        // (relics, ribbons, talismans, dishes, the Next Round button).
        // We pull the previous frame's snapshot here for both cursor
        // hit-testing and directional nav, mirroring the gameplay
        // scene's pattern.
        let focus_rects = self.last_focus_rects.borrow().clone();

        // Cursor-mode sync: when the player is using the mouse, hover IS
        // focus. We override `self.focus` from the renderer's pick path
        // first (which is precise — it raycasts against the actual 3D
        // mesh), and fall back to the projected 2D rect graph for the
        // Next Round button (which lives entirely in 2D).
        if ctx.input_mode == InputMode::Cursor {
            let (cx, cy) = ctx.cursor_pos;
            let new_focus = if let Some(hit) = ctx.picked_shop_object {
                Some(ShopFocus::from_hit(hit))
            } else {
                focus_target_at_cursor(&focus_rects, cx, cy)
            };
            self.focus = new_focus;
        }

        // Resolve the rect of the currently-focused element so the
        // spatial picker has a starting position.
        let current_focus_rect = self.focus.and_then(|t| {
            focus_rects
                .iter()
                .find_map(|(t2, r)| (*t2 == t).then_some(*r))
        });

        // Process directional + Confirm + Next-Round actions in one
        // pass so they cooperate cleanly with the existing
        // `CommitDiscard` shortcut and the SHOP_NEXT_ROUND_ID button.
        // CommitDiscard is left in place as the gameplay-style "always
        // advance" shortcut even when focus is on something else.
        for &a in ctx.actions {
            // Directional nav.
            let dir: Option<FocusDir> = match a {
                UiAction::FocusUp => Some(FocusDir::Up),
                UiAction::FocusDown => Some(FocusDir::Down),
                UiAction::FocusPrev => Some(FocusDir::Left),
                UiAction::FocusNext => Some(FocusDir::Right),
                _ => None,
            };
            if let Some(dir) = dir {
                let start_rect = current_focus_rect.or_else(|| {
                    // Seed on first directional press: prefer the first
                    // for-sale relic, else any first entry.
                    focus_rects
                        .iter()
                        .find_map(|(t, r)| matches!(t, ShopFocus::Relic(_)).then_some(*r))
                });
                if let Some(rect) = start_rect {
                    if let Some(next) = pick_neighbor(rect, dir, &focus_rects) {
                        self.focus = Some(next);
                    }
                } else if let Some((first, _)) = focus_rects.first() {
                    self.focus = Some(*first);
                }
                continue;
            }
            // Confirm: route by focused element. NextRound advances the
            // run; everything else fires the same action the click
            // dispatcher below would have fired for an equivalent mouse
            // click on the same target.
            if matches!(a, UiAction::Confirm) {
                if let Some(focus) = self.focus {
                    if matches!(focus, ShopFocus::NextRound) {
                        return Some(Scene::PickBlind(PickBlindScene::new()));
                    }
                    if let Some(hit) = focus.to_hit() {
                        if let Some(action) =
                            shop_action_for_hit(hit, &self.items, &self.consumable_items, ctx.run)
                        {
                            apply_shop_action(
                                action,
                                &mut self.items,
                                &mut self.consumable_items,
                                ctx.run,
                            );
                        } else if matches!(hit, ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK) {
                            // Journal book: same toggle as the click path.
                            self.journal.toggle();
                            return None;
                        }
                    }
                }
                continue;
            }
            // Cancel: clear focus so the next directional press re-seeds.
            if matches!(a, UiAction::Cancel) {
                self.focus = None;
                continue;
            }
        }

        // Enter / Next Round 2D button — kept as a global shortcut so
        // the legacy keybind still works regardless of where focus is.
        for a in ctx.actions {
            if matches!(a, UiAction::CommitDiscard) {
                return Some(Scene::PickBlind(PickBlindScene::new()));
            }
        }
        for &cid in ctx.button_clicks {
            if cid == SHOP_NEXT_ROUND_ID {
                return Some(Scene::PickBlind(PickBlindScene::new()));
            }
        }

        // 3D-hit dispatcher: when the catch-all click fires, route the
        // action based on what the renderer's pick path picked this frame.
        for &cid in ctx.button_clicks {
            if cid != SHOP_3D_HIT_ID {
                continue;
            }
            let Some(hit) = ctx.picked_shop_object else {
                continue;
            };
            // Journal book intercept — clicking it opens the Yaku
            // Journal overlay; the rest of update() bails out next
            // frame because `journal.open` will be true.
            if matches!(hit, ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK) {
                self.journal.toggle();
                return None;
            }
            if let Some(action) =
                shop_action_for_hit(hit, &self.items, &self.consumable_items, ctx.run)
            {
                apply_shop_action(action, &mut self.items, &mut self.consumable_items, ctx.run);
            }
            return None;
        }

        None
    }

    fn draw(&self, _ctx: DrawCtx<'_>) -> SceneDrawOutput {
        // Legacy fallback — the canonical path is `draw_frame()` below.
        SceneDrawOutput::default()
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let n_for_sale_ribbons = self.consumable_items.len();
        let n_owned_relics = ctx.run.relics.active.len();
        let n_fan = ctx.run.consumables.items.len();
        let layout = ShopLayout::build(
            ctx.layout,
            self.items.len(),
            n_for_sale_ribbons,
            n_owned_relics,
            n_fan,
        );

        let mut frame = UiFrame::new();
        // Pitch-black backdrop via the synthetic 1×1 black background
        // texture so the fill draws in pass A, *before* the smoke
        // composite. A fullscreen quad here would be reordered into the
        // late HUD overlay pass and paint over the smoke. The candle-lit
        // cabinet then pulls a warm pool of light out of the haze, reading
        // as "shop in a smoky backroom" rather than "game UI on a dark
        // page."
        frame.background(BackgroundId::Black);
        frame.camera_override = Some(layout.camera);

        // ── Curio cabinet (back wall) ──────────────────────────────────
        frame.curio_cabinet(CurioCabinetPlacement {
            center_pos: [
                layout.cabinet_pixel_x,
                layout.cabinet_pixel_y,
                layout.cabinet_world_y,
            ],
            extents: layout.cabinet_extents,
        });

        // ── Foreground dishes (relic + coin) ───────────────────────────
        frame.dish_explicit(DishExplicit {
            center_pos: [
                layout.relic_dish_center_px.0,
                layout.relic_dish_center_px.1,
                layout.relic_dish_center_px.2,
            ],
            extents: layout.relic_dish_extents,
            pick_id: Some(PICK_RELIC_DISH),
        });
        // Consumable tray — same visual language as the relic dish, just
        // a second compartment of the inventory shelf.
        frame.dish_explicit(DishExplicit {
            center_pos: [
                layout.consumable_row_center_px.0,
                layout.consumable_row_center_px.1,
                layout.consumable_row_center_px.2,
            ],
            extents: layout.consumable_row_extents,
            // No pick id — clicking individual consumables hits the
            // ribbon/talisman pick paths, not the tray itself.
            pick_id: None,
        });
        frame.dish_explicit(DishExplicit {
            center_pos: [
                layout.coin_dish_center_px.0,
                layout.coin_dish_center_px.1,
                layout.coin_dish_center_px.2,
            ],
            extents: layout.coin_dish_extents,
            pick_id: Some(PICK_COIN_DISH),
        });
        // Yaku Journal book — sits at the far-left of the inventory
        // shelf as a bookend to the player's possession area. Reuses the
        // dish pick path; clicking it opens `JournalOverlay`.
        let journal_cx = w * 0.13;
        let journal_cy = h * 0.70;
        let journal_cz = h * 0.05;
        let journal_extents = [w * 0.030, 22.0, h * 0.05];
        frame.dish_explicit(DishExplicit {
            center_pos: [journal_cx, journal_cy, journal_cz],
            extents: journal_extents,
            pick_id: Some(PICK_JOURNAL_BOOK),
        });

        // ── Relic batch: for-sale relics in cabinet niches, then owned ─
        // relics in the foreground dish. The order matters: pick_shop_object
        // returns indices into a flat list, so we partition with the
        // for-sale slots first and the owned slots second.
        let mut relic_placements: Vec<RelicPlacement> = Vec::new();
        // Relic half-extent expressed directly as a fraction of cabinet
        // width. The relics are the visual hero of the shop, so they want
        // to *own* their niche (~80% fill). Anchoring to cabinet width
        // (rather than mm) keeps them commanding at every resolution.
        let niche_base = layout.cabinet_extents[0] * 0.065;
        for (i, item) in self.items.iter().enumerate() {
            if i >= layout.niche_count {
                break;
            }
            let (px, py, wy) = layout.niche_centers_px[i];
            let half = relic_half_extents(item.relic, niche_base);
            let col = if item.sold {
                color::alpha(rarity_color(item.rarity), 0.35)
            } else {
                rarity_color(item.rarity)
            };
            relic_placements.push(RelicPlacement {
                world_pos: [px, py, wy],
                half_extents: half,
                color: col,
                relic_id: item.relic,
                glow: 0.0,
            });
        }
        let owned_base = 14.0_f32;
        for (i, &rid) in ctx.run.relics.active.iter().enumerate() {
            let (px, py, wy) = layout.owned_relic_pos(i);
            let rarity = all_relic_defs()
                .iter()
                .find(|d| d.id == rid)
                .map(|d| d.rarity)
                .unwrap_or(Rarity::Common);
            let half = relic_half_extents(rid, owned_base);
            relic_placements.push(RelicPlacement {
                world_pos: [px, py, wy],
                half_extents: half,
                color: rarity_color(rarity),
                relic_id: rid,
                glow: 0.0,
            });
        }
        if !relic_placements.is_empty() {
            frame.relic_batch(relic_placements);
        }

        // ── Consumable batches: zodiacs are silken ribbons, talismans are
        //    jade octagonal tablets. Each kind gets its own batch (and so
        //    its own pick path), but they share the same wall slot anchor
        //    positions to keep the layout simple.
        let mut ribbon_placements: Vec<ZodiacRibbonPlacement> = Vec::new();
        let mut talisman_placements: Vec<TalismanPlacement> = Vec::new();
        // For-sale: walk consumable_items, route each to its kind. Wall
        // slot is allocated by item index in self.consumable_items so
        // each item has a stable on-wall position.
        for (i, item) in self.consumable_items.iter().enumerate() {
            if i >= layout.ribbon_count {
                break;
            }
            let (ax, ay, awy) = layout.ribbon_anchors_px[i];
            let mut col = consumable_color(item.consumable);
            if item.sold {
                col[3] = 0.30;
            }
            match item.consumable {
                Consumable::Zodiac(z) => {
                    // Textured ribbons: white base so the silk texture
                    // shows through unmodified, but keep the per-item alpha
                    // (sold = 0.30) so spent slots dim out.
                    let alpha = col[3];
                    ribbon_placements.push(ZodiacRibbonPlacement {
                        anchor_pos: [ax, ay, awy],
                        length: layout.ribbon_length,
                        width: layout.ribbon_width,
                        rotation_y_deg: 0.0,
                        rotation_x_deg: 0.0,
                        color: [1.0, 1.0, 1.0, alpha],
                        kind: Some(z),
                    });
                }
                Consumable::Talisman(_) => {
                    // Talismans hover at the vertical mid-point of the
                    // ribbon row so they read as part of the same wall row.
                    talisman_placements.push(TalismanPlacement {
                        center_pos: [ax, ay, awy - layout.ribbon_length * 0.5],
                        extents: [
                            layout.ribbon_width * 1.4,
                            layout.ribbon_width * 2.0,
                            layout.ribbon_width * 0.35,
                        ],
                        rotation_y_deg: 0.0,
                        rotation_x_deg: 0.0,
                        color: col,
                    });
                }
            }
        }
        // Owned consumables: flat row in the consumable tray. Both
        // zodiac ribbons and talisman tablets lay face-up in the same
        // tray so they parse as one inventory section. Capped at 8 items
        // by `consumable_count` (the caller enforces the cap).
        for (i, c) in ctx.run.consumables.items.iter().enumerate() {
            if i >= layout.consumable_count {
                break;
            }
            let (ax, ay, awy) = layout.consumable_pos(i);
            match c {
                Consumable::Zodiac(z) => {
                    ribbon_placements.push(ZodiacRibbonPlacement {
                        anchor_pos: [ax, ay, awy],
                        length: layout.consumable_length,
                        width: layout.consumable_width,
                        rotation_y_deg: 0.0,
                        // Lay flat (face up) in the tray.
                        rotation_x_deg: -90.0,
                        color: [1.0, 1.0, 1.0, 1.0],
                        kind: Some(*z),
                    });
                }
                Consumable::Talisman(_) => {
                    talisman_placements.push(TalismanPlacement {
                        center_pos: [ax, ay, awy - layout.consumable_length * 0.4],
                        extents: [
                            layout.consumable_width * 1.4,
                            layout.consumable_width * 2.0,
                            layout.consumable_width * 0.35,
                        ],
                        rotation_y_deg: 0.0,
                        // Lay flat in the tray (face up).
                        rotation_x_deg: -90.0,
                        color: consumable_color(*c),
                    });
                }
            }
        }
        if !ribbon_placements.is_empty() {
            frame.zodiac_batch(ribbon_placements);
        }
        if !talisman_placements.is_empty() {
            frame.talisman_batch(talisman_placements);
        }

        // ── Coin pile inside the coin dish ─────────────────────────────
        let coins = coin_pile_layout(
            ctx.run.gold,
            layout.coin_dish_center_px,
            layout.coin_dish_extents,
            // Stable per-gold-count seed so the pile doesn't reshuffle each
            // frame.
            ctx.run.gold.wrapping_add(0xC01F).max(1),
        );
        if !coins.is_empty() {
            frame.coin_batch(coins);
        }

        // ── Smoky atmosphere ───────────────────────────────────────────
        // The fluid smoke pass renders curling volumetric haze across
        // the screen, depth-aware so it pools around the cabinet and
        // dishes. This is what sells the "shop in a backroom under a
        // dim lamp" mood — without it the scene reads as 3D objects on
        // a flat black UI page.
        frame.fluid_smoke();

        // ── Lighting: warm key + fill on cabinet, key on counter ───────
        let mut point_lights: Vec<PointLight> = vec![
            // Warm key on the curio cabinet from in front and above.
            PointLight {
                pos: [w * 0.5, h * 0.05, h * 0.55],
                radius: h * 1.20,
                color: [1.00, 0.86, 0.55],
                intensity: 1.85,
            },
            // Soft side fill on the cabinet's left half (relic niches).
            PointLight {
                pos: [w * 0.20, h * 0.20, h * 0.30],
                radius: h * 0.80,
                color: [1.00, 0.78, 0.46],
                intensity: 0.80,
            },
            // ── Inventory shelf spotlights ────────────────────────────
            // One warm key per zone on the shelf. Each light sits at
            // the same pixel-x as its tray but well *forward* in world
            // space (large world_y), so the light hangs above the shelf
            // like a track-mounted lamp instead of getting buried inside
            // the cabinet's wood mesh (which now spans most of the upper
            // pixel range after the handheld density pass).
            // Relic tray spotlight. Height kept inside the radius (see
            // the consumable spotlight comment below for the falloff math
            // — same bug, same fix).
            PointLight {
                pos: [
                    layout.relic_dish_center_px.0,
                    layout.relic_dish_center_px.1,
                    h * 0.20,
                ],
                radius: h * 0.65,
                color: [1.00, 0.88, 0.60],
                intensity: 2.80,
            },
            // Consumable tray spotlight — lights the player's owned
            // zodiac ribbons / talisman tablets laid flat in the tray.
            //
            // CRITICAL: lit_mesh.wgsl attenuation is `(1 - dist/radius)²`
            // and clamps to zero past `radius`. The tray surface sits at
            // world_y ≈ 0 (a few scaled-mm above), so a light hovering at
            // `h * 0.95` with a `h * 0.70` radius is *outside* its own
            // reach (dist 1026 > radius 756) and contributes literally
            // nothing — which is why the items read as if the light were
            // *below* the tray. Drop the height to `h * 0.18` so the
            // light actually lives inside its falloff envelope, and
            // widen the radius so the entire 8-slot row is inside it.
            PointLight {
                pos: [
                    layout.consumable_row_center_px.0,
                    layout.consumable_row_center_px.1,
                    h * 0.18,
                ],
                radius: h * 0.60,
                color: [1.00, 0.90, 0.62],
                intensity: 3.00,
            },
            // Coin dish spotlight — tighter, slightly cooler so the gold
            // pops without overpowering the relic tray. Height kept
            // inside the radius (same falloff fix as above).
            PointLight {
                pos: [
                    layout.coin_dish_center_px.0,
                    layout.coin_dish_center_px.1,
                    h * 0.14,
                ],
                radius: h * 0.42,
                color: [1.00, 0.92, 0.66],
                intensity: 2.60,
            },
            // Journal book spotlight — small, anchored to the bookend
            // at the far-left of the shelf so it reads as its own object.
            // Height kept inside the radius (same falloff fix as above).
            PointLight {
                pos: [journal_cx, journal_cy, h * 0.10],
                radius: h * 0.30,
                color: [1.00, 0.90, 0.64],
                intensity: 2.40,
            },
        ];

        // ── Hover spotlight: literal point light on the picked object ──
        // Uses the renderer's pick result so the spotlight is anchored to
        // the actual visible object the cursor is over.
        // Hover follows focus first (so controller / keyboard players see
        // the spotlight + tooltip on whatever they've navigated to), with
        // the cursor pick acting as a fallback for first-frame mouse
        // hovers before update() has had a chance to sync focus to the
        // cursor. Cursor mode `update()` writes `self.focus` from the
        // pick result already, so this expression collapses to "show the
        // focused element" in the steady state.
        let hover: Option<ShopHit> = self
            .focus
            .and_then(|f| f.to_hit())
            .or(ctx.picked_shop_object);
        if let Some(hit) = hover {
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            // Helper: get the (px, py, wy) anchor of a hit consumable for
            // spotlight placement. Walks the same partition the renderer
            // uses (for-sale-of-kind, then owned-of-kind) to find which
            // wall slot or fan position to light up.
            let consumable_anchor = |is_zodiac: bool, hit_idx: usize| -> Option<(f32, f32, f32)> {
                let mut for_sale_count = 0usize;
                for (slot_i, item) in self.consumable_items.iter().enumerate() {
                    let matches = match item.consumable {
                        Consumable::Zodiac(_) => is_zodiac,
                        Consumable::Talisman(_) => !is_zodiac,
                    };
                    if !matches {
                        continue;
                    }
                    if for_sale_count == hit_idx {
                        if slot_i < layout.ribbon_count {
                            return Some(layout.ribbon_anchors_px[slot_i]);
                        }
                        return None;
                    }
                    for_sale_count += 1;
                }
                // Owned-tray section.
                let owned_target = hit_idx - for_sale_count;
                let mut owned_count = 0usize;
                for (row_i, c) in ctx.run.consumables.items.iter().enumerate() {
                    let matches = match c {
                        Consumable::Zodiac(_) => is_zodiac,
                        Consumable::Talisman(_) => !is_zodiac,
                    };
                    if !matches {
                        continue;
                    }
                    if owned_count == owned_target {
                        if row_i < layout.consumable_count {
                            return Some(layout.consumable_pos(row_i));
                        }
                        return None;
                    }
                    owned_count += 1;
                }
                None
            };
            match hit {
                ShopHit::Relic(i) => {
                    let (px, py, wy) = if i < n_for_sale_relics {
                        layout.niche_centers_px[i]
                    } else {
                        let oi = i - n_for_sale_relics;
                        if oi < n_owned_relics {
                            layout.owned_relic_pos(oi)
                        } else {
                            (w * 0.5, h * 0.5, 0.0)
                        }
                    };
                    point_lights.push(PointLight {
                        pos: [px, py - 30.0, wy + 60.0],
                        radius: 180.0,
                        color: [1.00, 0.92, 0.70],
                        intensity: 3.20,
                    });
                }
                ShopHit::Ribbon(i) => {
                    if let Some((px, py, wy)) = consumable_anchor(true, i) {
                        point_lights.push(PointLight {
                            pos: [px, py + 40.0, wy - layout.ribbon_length * 0.4],
                            radius: 200.0,
                            color: [1.00, 0.92, 0.74],
                            intensity: 3.00,
                        });
                    }
                }
                ShopHit::Talisman(i) => {
                    if let Some((px, py, wy)) = consumable_anchor(false, i) {
                        point_lights.push(PointLight {
                            pos: [px, py + 30.0, wy - layout.ribbon_length * 0.5],
                            radius: 180.0,
                            color: [0.78, 1.00, 0.82],
                            intensity: 3.20,
                        });
                    }
                }
                ShopHit::Dish(id) => {
                    let center = if id == PICK_RELIC_DISH {
                        layout.relic_dish_center_px
                    } else if id == PICK_JOURNAL_BOOK {
                        (journal_cx, journal_cy, journal_cz)
                    } else {
                        layout.coin_dish_center_px
                    };
                    point_lights.push(PointLight {
                        pos: [center.0, center.1 - 20.0, 80.0],
                        radius: 220.0,
                        color: [1.00, 0.92, 0.70],
                        intensity: 2.50,
                    });
                }
            }
        }
        frame.point_lights = point_lights;

        // ── Hanging shop plaque (3D) ───────────────────────────────────
        // The 3D mesh + cord strands + title text are pushed directly
        // into `frame.cmds` here (before the 2D HUD section starts
        // collecting local quads/texts), so the plaque sits at the very
        // bottom of the 2D z-order — every subsequent button, tooltip,
        // pause menu, modal, or glossary popup renders on top of it.
        // (Putting it in the local `texts` vec used to leak above
        // tooltips because that vec gets flushed AFTER the global
        // tooltip overlay's quads in some draw paths.)
        let plaque_world_y = layout.cabinet_world_y + layout.cabinet_extents[1] * 0.55;
        let plaque_pixel_x = w * 0.5;
        let plaque_pixel_y = layout.cabinet_pixel_y + layout.cabinet_extents[2] * 0.6;
        let plaque_world_w = (w * 0.34).clamp(260.0, 480.0);
        let plaque_world_h = (h * 0.11).clamp(72.0, 120.0);
        let plaque_world_t = 10.0_f32;
        frame.plaque(PlaquePlacement {
            center_pos: [plaque_pixel_x, plaque_pixel_y, plaque_world_y],
            extents: [plaque_world_w, plaque_world_h, plaque_world_t],
            rotation_y_deg: 0.0,
            top_text: format!("SHOP  ·  Round {}", self.came_from_round),
            bot_text: String::new(),
        });
        // Previous-frame projection of the plaque face (1-frame lag,
        // same pattern as relic/ribbon rects). Empty on the first frame
        // — fall back to a centered estimate so the title still appears.
        let plaque_screen = ctx
            .projected_plaque_rects
            .first()
            .copied()
            .unwrap_or_else(|| {
                let est_w = w * 0.30;
                let est_h = h * 0.08;
                [(w - est_w) * 0.5, h * 0.05, est_w, est_h]
            });
        // Suspension cords from ceiling to the projected chain-nub
        // positions on the top edge of the plaque rect.
        let cord_inset = plaque_screen[2] * 0.12;
        let cord_left = plaque_screen[0] + cord_inset;
        let cord_right = plaque_screen[0] + plaque_screen[2] - cord_inset;
        let cord_top_y = plaque_screen[1].max(0.0);
        for &cx in &[cord_left, cord_right] {
            frame.quad(GpuInstance {
                rect: [cx - 2.5, 0.0, 5.0, cord_top_y],
                color: [0.16, 0.10, 0.05, 1.0],
            });
            frame.quad(GpuInstance {
                rect: [cx - 0.75, 0.0, 1.5, (cord_top_y - 2.0).max(0.0)],
                color: [0.34, 0.22, 0.10, 1.0],
            });
        }

        // ── 2D HUD: tooltip + chrome buttons ───────────────────────────
        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut texts: Vec<TextLabel> = Vec::new();
        let mut buttons: Vec<ButtonDef> = Vec::new();
        // Suppress unused-binding warning when score_panel was the previous
        // anchor and is no longer used here.
        let _ = ctx.layout.score_panel;

        // Tooltip on the hovered object.
        if let Some(hit) = hover {
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            // Helper: walk consumable_items and find the i-th item of the
            // requested kind (zodiac or talisman). Same partition rule the
            // renderer uses to assign hit indices.
            let nth_consumable_of_kind = |is_zodiac: bool, hit_idx: usize| -> Option<usize> {
                let mut count = 0usize;
                for (slot_i, item) in self.consumable_items.iter().enumerate() {
                    let matches = match item.consumable {
                        Consumable::Zodiac(_) => is_zodiac,
                        Consumable::Talisman(_) => !is_zodiac,
                    };
                    if !matches {
                        continue;
                    }
                    if count == hit_idx {
                        return Some(slot_i);
                    }
                    count += 1;
                }
                None
            };
            let n_for_sale_zodiacs = self
                .consumable_items
                .iter()
                .filter(|c| matches!(c.consumable, Consumable::Zodiac(_)))
                .count();
            let n_for_sale_talismans = self
                .consumable_items
                .iter()
                .filter(|c| matches!(c.consumable, Consumable::Talisman(_)))
                .count();
            // Look up the projected screen rect from the renderer.
            let tooltip_anchor: Option<[f32; 4]> = match hit {
                ShopHit::Relic(i) => ctx.projected_relic_rects.get(i).copied(),
                ShopHit::Ribbon(i) => ctx.projected_ribbon_rects.get(i).copied(),
                ShopHit::Talisman(i) => ctx.projected_talisman_rects.get(i).copied(),
                ShopHit::Dish(id) => ctx
                    .aux_dish_rects
                    .iter()
                    .find_map(|(pid, r)| if *pid == Some(id) { Some(*r) } else { None }),
            };
            // Build the tooltip body lines based on hit kind.
            let (title, subtitle, cta, cta_color) = match hit {
                ShopHit::Relic(i) if i < n_for_sale_relics => {
                    let item = &self.items[i];
                    let can_afford =
                        ctx.run.gold >= item.price && !ctx.run.relics.is_full() && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        if ctx.run.relics.is_full() {
                            "Relics full".to_string()
                        } else {
                            format!("Need {}g", item.price - ctx.run.gold)
                        }
                    } else {
                        format!("Buy {}g", item.price)
                    };
                    let col = if item.sold {
                        color::SLATE
                    } else if can_afford {
                        color::GOLD
                    } else {
                        color::RUBY
                    };
                    (
                        item.name.to_string(),
                        item.description.to_string(),
                        cta,
                        col,
                    )
                }
                ShopHit::Relic(i) => {
                    let oi = i - n_for_sale_relics;
                    if oi < ctx.run.relics.active.len() {
                        let rid = ctx.run.relics.active[oi];
                        let defs = all_relic_defs();
                        let def = defs.iter().find(|d| d.id == rid);
                        let (name, desc) = def
                            .map(|d| (d.name.to_string(), d.description.to_string()))
                            .unwrap_or(("Relic".into(), String::new()));
                        (
                            name,
                            desc,
                            format!("Sell {}g", relic_sell_price(rid)),
                            color::CHAMPAGNE,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Ribbon(i) if i < n_for_sale_zodiacs => {
                    if let Some(slot_i) = nth_consumable_of_kind(true, i) {
                        let item = &self.consumable_items[slot_i];
                        let price = item.price();
                        let can_afford =
                            ctx.run.gold >= price && !ctx.run.consumables.is_full() && !item.sold;
                        let cta = if item.sold {
                            "SOLD".to_string()
                        } else if !can_afford {
                            if ctx.run.consumables.is_full() {
                                "Inventory full".to_string()
                            } else {
                                format!("Need {}g", price - ctx.run.gold)
                            }
                        } else {
                            format!("Buy {}g", price)
                        };
                        let col = if item.sold {
                            color::SLATE
                        } else if can_afford {
                            color::GOLD
                        } else {
                            color::RUBY
                        };
                        (item.name(), item.description(), cta, col)
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Ribbon(i) => {
                    // Owned zodiac in the fan.
                    let oi = i - n_for_sale_zodiacs;
                    let mut owned_count = 0usize;
                    let mut found = None;
                    for c in ctx.run.consumables.items.iter() {
                        if matches!(c, Consumable::Zodiac(_)) {
                            if owned_count == oi {
                                found = Some(*c);
                                break;
                            }
                            owned_count += 1;
                        }
                    }
                    if let Some(c) = found {
                        let item = ConsumableShopItem {
                            consumable: c,
                            sold: false,
                        };
                        (
                            item.name(),
                            item.description(),
                            format!("Sell {}g", consumable_sell_price(c)),
                            color::CHAMPAGNE,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Talisman(i) if i < n_for_sale_talismans => {
                    if let Some(slot_i) = nth_consumable_of_kind(false, i) {
                        let item = &self.consumable_items[slot_i];
                        let price = item.price();
                        let can_afford =
                            ctx.run.gold >= price && !ctx.run.consumables.is_full() && !item.sold;
                        let cta = if item.sold {
                            "SOLD".to_string()
                        } else if !can_afford {
                            if ctx.run.consumables.is_full() {
                                "Inventory full".to_string()
                            } else {
                                format!("Need {}g", price - ctx.run.gold)
                            }
                        } else {
                            format!("Buy {}g", price)
                        };
                        let col = if item.sold {
                            color::SLATE
                        } else if can_afford {
                            color::GOLD
                        } else {
                            color::RUBY
                        };
                        (item.name(), item.description(), cta, col)
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Talisman(i) => {
                    // Owned talisman in the fan.
                    let oi = i - n_for_sale_talismans;
                    let mut owned_count = 0usize;
                    let mut found = None;
                    for c in ctx.run.consumables.items.iter() {
                        if matches!(c, Consumable::Talisman(_)) {
                            if owned_count == oi {
                                found = Some(*c);
                                break;
                            }
                            owned_count += 1;
                        }
                    }
                    if let Some(c) = found {
                        let item = ConsumableShopItem {
                            consumable: c,
                            sold: false,
                        };
                        (
                            item.name(),
                            item.description(),
                            format!("Sell {}g", consumable_sell_price(c)),
                            color::CHAMPAGNE,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Dish(id) if id == PICK_COIN_DISH => (
                    "Gold".to_string(),
                    "Your current treasure".to_string(),
                    format!("{}g", ctx.run.gold),
                    color::GOLD,
                ),
                ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK => (
                    "Yaku Journal".to_string(),
                    "Levels, plays, and how to build every yaku".to_string(),
                    "Open".to_string(),
                    color::CHAMPAGNE,
                ),
                ShopHit::Dish(_) => (
                    "Relic dish".to_string(),
                    "Hover an owned relic to sell it".to_string(),
                    String::new(),
                    color::SLATE,
                ),
            };
            if !title.is_empty() {
                if let Some(rect) = tooltip_anchor {
                    let pad = 16.0_f32;
                    // Pin font sizes explicitly (don't let the rasterizer
                    // auto-shrink) so they match what the typography tiers
                    // are supposed to produce at this window height.
                    let title_font = typography::size(typography::TITLE, h).max(22.0);
                    let body_font = typography::size(typography::BODY, h).max(16.0);
                    let cta_font = typography::size(typography::HEADING, h).max(20.0);
                    let title_h = title_font * 1.4;
                    let cta_h = cta_font * 1.5;
                    let body_line_step = body_font * 1.4;

                    // Size the tooltip to fit its content: wrap the body
                    // first, then grow the panel to the wrapped line count
                    // so descriptions never get truncated mid-sentence.
                    let tip_w = (w * 0.36).clamp(360.0, 560.0);
                    let body_inner_w = tip_w - pad * 2.0;
                    let wrapped = widget::wrap_text(&subtitle, body_inner_w, body_font);
                    let body_lines = wrapped.len().max(1) as f32;
                    let body_h =
                        (body_lines * body_line_step + body_line_step * 0.4).max(body_line_step);
                    let needed_h = pad * 2.0 + title_h + 6.0 + body_h + 12.0 + cta_h;
                    let tip_h = needed_h.min(h - 32.0).max(180.0);
                    // Anchor above the projected rect, clamped to screen.
                    let mut tip_x = rect[0] + rect[2] * 0.5 - tip_w * 0.5;
                    let mut tip_y = rect[1] - tip_h - 16.0;
                    if tip_y < 8.0 {
                        tip_y = rect[1] + rect[3] + 16.0;
                    }
                    if tip_y + tip_h > h - 8.0 {
                        tip_y = (h - tip_h - 8.0).max(8.0);
                    }
                    if tip_x < 8.0 {
                        tip_x = 8.0;
                    }
                    if tip_x + tip_w > w - 8.0 {
                        tip_x = w - tip_w - 8.0;
                    }
                    widget::push_panel(
                        &mut quads,
                        [tip_x, tip_y, tip_w, tip_h],
                        PanelVariant::Hero,
                    );
                    let body_y = tip_y + pad + title_h + 6.0;

                    // Title
                    texts.push(TextLabel {
                        rect: [tip_x + pad, tip_y + pad, tip_w - pad * 2.0, title_h],
                        text: title,
                        color: color::CHAMPAGNE,
                        font_px: Some(title_font),
                        align: TextAlign::Left,
                        ..Default::default()
                    });
                    // Body — wrap so long descriptions stay legible.
                    widget::push_text_block(
                        &mut texts,
                        [tip_x + pad, body_y, tip_w - pad * 2.0, body_h],
                        &subtitle,
                        TextStyle {
                            tier: typography::BODY,
                            color: color::PARCHMENT,
                            padding: 0.0,
                            align: TextAlign::Left,
                        },
                        h,
                    );
                    // CTA — bottom-right.
                    if !cta.is_empty() {
                        texts.push(TextLabel {
                            rect: [
                                tip_x + pad,
                                tip_y + tip_h - cta_h - pad * 0.5,
                                tip_w - pad * 2.0,
                                cta_h,
                            ],
                            text: cta,
                            color: cta_color,
                            font_px: Some(cta_font),
                            align: TextAlign::Right,
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // ── Next Round button (always-visible, 2D) ─────────────────────
        let scale = (w.min(h)) / 600.0;
        let btn_w = (180.0 * scale).max(120.0);
        let btn_h = (44.0 * scale).max(28.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (16.0 * scale);
        widget::push_button(
            &mut quads,
            &mut texts,
            &mut buttons,
            [btn_x, btn_y, btn_w, btn_h],
            "Next Round",
            ButtonVariant::Primary,
            ButtonState::Rest,
            UiAction::CommitDiscard,
        );
        // Replace the synthetic UiAction button push_button registered with
        // a Scene-id button so we can route it through update().
        buttons.pop();
        buttons.push(ButtonDef::scene(
            (btn_x, btn_y, btn_w, btn_h),
            SHOP_NEXT_ROUND_ID,
        ));
        let next_round_rect: [f32; 4] = [btn_x, btn_y, btn_w, btn_h];

        // ── Focus rect graph + brass focus ring ────────────────────────
        //
        // Build a single list of `(ShopFocus, screen_rect)` covering
        // every navigable shop element this frame, then stash it for
        // `update()` to consume next frame. Same one-frame-stale pattern
        // the gameplay scene uses.
        //
        // Source rects:
        //   - Relics: `projected_relic_rects` (in-cabinet then in-dish
        //     order, matching the renderer's flat list)
        //   - Ribbons: `projected_ribbon_rects`
        //   - Talismans: `projected_talisman_rects`
        //   - Dishes: `aux_dish_rects` paired with their pick id
        //   - Next Round: the 2D button rect we just computed above
        let mut focus_rect_graph: Vec<(ShopFocus, [f32; 4])> = Vec::new();
        for (i, r) in ctx.projected_relic_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((ShopFocus::Relic(i), *r));
            }
        }
        for (i, r) in ctx.projected_ribbon_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((ShopFocus::Ribbon(i), *r));
            }
        }
        for (i, r) in ctx.projected_talisman_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((ShopFocus::Talisman(i), *r));
            }
        }
        for (pid, r) in ctx.aux_dish_rects.iter() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                if let Some(id) = pid {
                    focus_rect_graph.push((ShopFocus::Dish(*id), *r));
                }
            }
        }
        focus_rect_graph.push((ShopFocus::NextRound, next_round_rect));

        // Push the brass focus ring on top of the 2D HUD layer so it
        // sits above the cabinet wood and dishes. Skipped during pause /
        // overlay states because the overlay's own buttons take focus.
        if !self.pause_menu.paused && !self.glossary.open && !self.journal.open {
            if let Some(target) = self.focus {
                let rect_lookup = focus_rect_graph
                    .iter()
                    .find_map(|(t, r)| (*t == target).then_some(*r));
                if let Some(rect) = rect_lookup {
                    push_focus_ring(rect, scale, &mut quads);
                }
            }
        }

        // Stash for next frame's update().
        *self.last_focus_rects.borrow_mut() = focus_rect_graph;

        // The `?` glossary badge has been removed — the glossary is
        // reachable from the pause menu's "Glossary" entry. The keyboard
        // `Help` action shortcut still works for power users.

        // ── Catch-all 3D-hit dispatcher ───────────────────────────────
        // Full-screen button registered LAST so it only wins if no other
        // (smaller) button matched the cursor first.
        buttons.push(ButtonDef::scene((0.0, 0.0, w, h), SHOP_3D_HIT_ID));

        // Pause overlay. While paused, drop all shop buttons (next-round,
        // help badge, full-screen 3D catch-all, etc.) so the pause menu's
        // own buttons are the only clickable surfaces — otherwise the
        // SHOP_3D_HIT_ID full-screen catch-all above would intercept every
        // click before the pause buttons even get tested.
        if self.pause_menu.paused {
            buttons.clear();
        }
        self.pause_menu
            .draw(w, h, scale, &mut quads, &mut texts, &mut buttons);
        // Fullscreen click-blocker behind the pause menu's own buttons so
        // missed clicks become no-ops instead of falling through.
        if self.pause_menu.paused {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Glossary overlay (drawn last so it covers everything).
        self.glossary
            .draw(w, h, &mut quads, &mut texts, &mut buttons);
        if self.glossary.open {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Yaku Journal overlay — drawn after the glossary so opening
        // both at once would let the journal win, mirroring the
        // glossary-vs-pause precedence.
        self.journal
            .draw(w, h, ctx.run, &mut quads, &mut texts, &mut buttons);
        if self.journal.open {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Push 2D layers onto the frame after all 3D content.
        frame.quads(quads);
        frame.texts(texts);
        frame.buttons = buttons;
        frame.window_title = format!(
            "Mahjuro — Shop (Round {}) — Gold: {}",
            self.came_from_round, ctx.run.gold
        );

        frame
    }
}
