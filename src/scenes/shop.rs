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

use std::time::Instant;

use crate::core::consumable::Consumable;
use crate::core::relic::{
    Rarity, RelicId, RelicState, all_relic_defs, relic_description_live, relic_sell_price,
    relic_sell_price_live, relic_shop_price,
};
use crate::core::talisman::TalismanKind;
use crate::core::tile::Tile;
use crate::core::tile_pack::TilePackKind;
use crate::core::zodiac::ZodiacKind;
use crate::game::onboarding::OnboardingPhase;
use crate::render::curio_cabinet_mesh::{NICHE_COLS, NICHE_ROWS, niche_centers_local};
use crate::render::draw_cmd::{
    BookPlacement, CameraParams, CoinPlacement, CurioCabinetPlacement, DishExplicit,
    GoldBarPlacement, PackPlacement, PlaquePlacement, RelicPlacement, ShowcaseTilePlacement,
    TalismanPlacement, UiFrame, ZodiacRibbonPlacement,
};
use crate::render::particles::ParticleSystem;
use crate::render::score_popups::ScorePopupSystem;
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, ShopHit, TextAlign, TextLabel};
use crate::ui::focus_nav::{FocusDir, focus_target_at_cursor, pick_neighbor, push_focus_ring};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::widget::{self, PanelVariant, TextStyle};

use super::pause_menu::PauseMenu;
use super::pick_blind::PickBlindScene;
use super::{
    BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShopAction {
    BuyCard(usize),
    SellRelic(usize),
    BuyZodiac(usize),
    BuyTalisman(usize),
    /// Sell the consumable at this index in `run.consumables.items`.
    SellConsumable(usize),
    /// Use the consumable at this index in `run.consumables.items`.
    /// Currently only zodiacs can be used from the shop (levels up a yaku).
    UseConsumable(usize),
    BuyPack,
    #[allow(dead_code)]
    Reroll,
    /// Swap an owned relic one slot to the left.
    MoveRelicLeft(usize),
    /// Swap an owned relic one slot to the right.
    MoveRelicRight(usize),
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
    /// The for-sale tile packs (if any).
    Pack(u32),
    /// The 2D "Next Round" button at the bottom of the screen.
    NextRound,
    /// The 2D "Reroll" button at the bottom of the screen.
    Reroll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShopMode {
    Standard,
    Tutorial,
}

impl ShopFocus {
    fn from_hit(h: ShopHit) -> Self {
        match h {
            ShopHit::Relic(i) => Self::Relic(i),
            ShopHit::Ribbon(i) => Self::Ribbon(i),
            ShopHit::Talisman(i) => Self::Talisman(i),
            ShopHit::Dish(id) => Self::Dish(id),
            ShopHit::TilePack(id) => Self::Pack(id),
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
            Self::Pack(id) => Some(ShopHit::TilePack(id)),
            Self::NextRound | Self::Reroll => None,
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
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    run: &crate::game::run::RunState,
) -> Option<ShopAction> {
    let n_for_sale = items.len();
    match hit {
        ShopHit::Relic(i) => {
            if i < n_for_sale {
                Some(ShopAction::BuyCard(i))
            } else {
                None
            }
        }
        ShopHit::Ribbon(i) => {
            if i < zodiac_items.len() {
                Some(ShopAction::BuyZodiac(i))
            } else {
                let oi = i - zodiac_items.len();
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
                inv_idx.map(ShopAction::UseConsumable)
            }
        }
        ShopHit::Talisman(i) => {
            if i < talisman_items.len() {
                Some(ShopAction::BuyTalisman(i))
            } else {
                None
            }
        }
        ShopHit::Dish(id) if id == PICK_TILE_PACK => Some(ShopAction::BuyPack),
        ShopHit::Dish(_) => None,
        ShopHit::TilePack(_) => Some(ShopAction::BuyPack),
    }
}

/// Result of applying a shop action — tells the caller what visual
/// feedback to show.
enum ShopActionResult {
    /// Nothing special to show.
    None,
    /// A tile pack was purchased — show the opening celebration.
    PackCelebration(PackCelebration),
    /// A zodiac was purchased and instantly applied — show the close-up
    /// celebration overlay, then the level-up popup.
    ZodiacApplied {
        zodiac_kind: ZodiacKind,
        yaku_name: &'static str,
        new_level: u32,
    },
}

/// Apply a `ShopAction` to the shop's stock + the player's run state.
/// Pulled out of the inline click-dispatch code so the focus model's
/// `Confirm` path can fire the same action handlers without
/// duplication.
fn apply_shop_action(
    action: ShopAction,
    items: &mut Vec<ShopItem>,
    zodiac_items: &mut Vec<ConsumableShopItem>,
    talisman_items: &mut Vec<ConsumableShopItem>,
    pack_item: &mut Option<TilePackShopItem>,
    run: &mut crate::game::run::RunState,
    bus: &mut crate::game::event_bus::EventBus,
) -> ShopActionResult {
    match action {
        ShopAction::BuyCard(idx) => {
            if idx < items.len() {
                let item = &items[idx];
                if !item.sold && run.gold >= item.price as i32 && !run.relics.is_full() {
                    let price = item.price;
                    let relic = item.relic;
                    run.gold -= price as i32;
                    run.relics.active.push(relic);
                    // Initialize counters for stateful relics.
                    match relic {
                        RelicId::MeltingIce => {
                            run.relic_counters.insert(RelicId::MeltingIce, 80);
                        }
                        RelicId::SilkThread => {
                            run.relic_counters.insert(RelicId::SilkThread, 40);
                        }
                        RelicId::TeaCeremony => {
                            run.relic_counters.insert(RelicId::TeaCeremony, 3);
                        }
                        _ => {}
                    }
                    run.recompute_capacities();
                    items.remove(idx);
                    // Tutorial milestone: celebrate the first shop purchase.
                    if let Some(ref mut tut) = run.tutorial {
                        if tut.celebrate(crate::game::tutorial::TutorialMilestone::FirstShopBuy) {
                            bus.push(crate::game::event_bus::GameEvent::TutorialMilestone(
                                crate::game::tutorial::TutorialMilestone::FirstShopBuy,
                            ));
                        }
                    }
                }
            }
        }
        ShopAction::SellRelic(idx) => {
            if idx < run.relics.active.len() {
                let rid = run.relics.active[idx];
                let mut refund = relic_sell_price(rid);
                // Smoke Bomb: sell to skip the upcoming boss blind.
                if rid == RelicId::SmokeBomb
                    && run.upcoming_blind == crate::core::rules::BlindKind::Boss
                {
                    run.relics.active.remove(idx);
                    // Skip boss — advance the ante without playing.
                    run.ante += 1;
                    run.base_target = (run.base_target as f32 * run.mode.target_scaling) as u32;
                    run.upcoming_blind = crate::core::rules::BlindKind::Small;
                    return ShopActionResult::None;
                }
                // Nest Egg: sell value grows by +2 per round held.
                if rid == RelicId::NestEgg {
                    let rounds = run
                        .relic_counters
                        .get(&RelicId::NestEgg)
                        .copied()
                        .unwrap_or(0);
                    refund = refund.saturating_add(2 * rounds as u32);
                }
                // Phantom Relic: after 3 rounds, selling duplicates a
                // random owned relic instead of giving gold.
                if rid == RelicId::PhantomRelic {
                    let rounds = run
                        .relic_counters
                        .get(&RelicId::PhantomRelic)
                        .copied()
                        .unwrap_or(0);
                    run.relics.active.remove(idx);
                    run.relic_counters.remove(&RelicId::PhantomRelic);
                    if rounds >= 3 && !run.relics.is_full() {
                        use rand::seq::IndexedRandom;
                        let mut rng = rand::rng();
                        if let Some(&dupe) = run.relics.active.choose(&mut rng) {
                            run.relics.active.push(dupe);
                        }
                    } else {
                        run.gold = run.gold.saturating_add(refund as i32);
                    }
                    return ShopActionResult::None;
                }
                // Ritual Blade: destroy the relic to the right and gain
                // permanent mult equal to double its sell value.
                if rid == RelicId::RitualBlade && idx + 1 < run.relics.active.len() {
                    let victim_id = run.relics.active[idx + 1];
                    let victim_value = relic_sell_price(victim_id) as i32;
                    // Remove victim first (it's at idx+1), then blade (at idx).
                    run.relics.active.remove(idx + 1);
                    run.relics.active.remove(idx);
                    // Store permanent mult as ×10 integer.
                    *run.relic_counters.entry(RelicId::RitualBlade).or_insert(0) +=
                        victim_value * 2 * 10;
                    run.relic_activations.push(RelicId::RitualBlade);
                    return ShopActionResult::None;
                }
                run.relics.active.remove(idx);
                run.gold = run.gold.saturating_add(refund as i32);
                // Bonfire: track relic sales for mult bonus.
                *run.relic_counters.entry(RelicId::Bonfire).or_insert(0) += 1;
                if run.relics.has(RelicId::Bonfire) {
                    run.relic_activations.push(RelicId::Bonfire);
                }
            }
        }
        ShopAction::MoveRelicLeft(idx) => {
            if idx > 0 {
                run.relics.swap_relics(idx, idx - 1);
            }
        }
        ShopAction::MoveRelicRight(idx) => {
            if idx + 1 < run.relics.active.len() {
                run.relics.swap_relics(idx, idx + 1);
            }
        }
        ShopAction::BuyZodiac(idx) => {
            if idx < zodiac_items.len() {
                let item = &zodiac_items[idx];
                let price = item.price();
                if !item.sold && run.gold >= price as i32 {
                    if let Consumable::Zodiac(z) = item.consumable {
                        run.gold -= price as i32;
                        let yaku = z.yaku();
                        let new_level = run.yaku_levels.level_up(yaku);
                        zodiac_items.remove(idx);
                        return ShopActionResult::ZodiacApplied {
                            zodiac_kind: z,
                            yaku_name: yaku.name(),
                            new_level,
                        };
                    }
                }
            }
        }
        ShopAction::BuyTalisman(idx) => {
            if idx < talisman_items.len() {
                let item = &talisman_items[idx];
                let price = item.price();
                if !item.sold && run.gold >= price as i32 && !run.consumables.is_full() {
                    let consumable = item.consumable;
                    run.gold -= price as i32;
                    run.consumables.items.push(consumable);
                    talisman_items.remove(idx);
                }
            }
        }
        ShopAction::SellConsumable(idx) => {
            if idx < run.consumables.items.len() {
                let c = run.consumables.items[idx];
                let refund = consumable_sell_price(c);
                run.consumables.items.remove(idx);
                run.gold = run.gold.saturating_add(refund as i32);
            }
        }
        ShopAction::UseConsumable(idx) => {
            if idx < run.consumables.items.len() {
                if let Consumable::Zodiac(z) = run.consumables.items[idx] {
                    if let Some(crate::game::run::ConsumableUseResult::Zodiac { yaku, new_level }) =
                        run.use_consumable(idx)
                    {
                        return ShopActionResult::ZodiacApplied {
                            zodiac_kind: z,
                            yaku_name: yaku.name(),
                            new_level,
                        };
                    }
                }
            }
        }
        ShopAction::BuyPack => {
            if let Some(ref mut pack) = *pack_item {
                if !pack.sold {
                    let price = pack.kind.shop_price();
                    if run.gold >= price as i32 {
                        run.gold -= price as i32;
                        // Generate the tiles that will enter the wall.
                        use crate::core::tile_pack::{PACK_ID_STRIDE, PACK_TILE_ID_BASE};
                        let pack_idx = run.tile_packs.len();
                        let start_id = PACK_TILE_ID_BASE + (pack_idx as u32) * PACK_ID_STRIDE;
                        let mut tiles = pack.kind.generate_tiles(start_id);
                        // Stamp pre-enhancements (e.g. Polychrome) before
                        // pushing the pack so IDs are deterministic.
                        if let Some(enh) = pack.kind.pre_enhancement() {
                            for t in &mut tiles {
                                run.tile_enhancements.insert(t.id, enh);
                                t.enhancement = Some(enh);
                            }
                        }
                        run.tile_packs.push(pack.kind);
                        pack.sold = true;
                        bus.push(crate::game::event_bus::GameEvent::PackBought);
                        let kind = pack.kind;
                        let name = pack.kind.name();
                        return ShopActionResult::PackCelebration(PackCelebration::new(
                            tiles, name, kind,
                        ));
                    }
                }
            }
            return ShopActionResult::None;
        }
        // Reroll is handled directly in ShopScene::update() because it
        // needs `&mut self` — this arm keeps the match exhaustive.
        ShopAction::Reroll => {}
    }
    ShopActionResult::None
}

/// Refund when selling a consumable — half buy price, minimum 1 gold,
/// matching the relic sell formula.
fn consumable_sell_price(c: Consumable) -> u32 {
    let buy = match c {
        Consumable::Zodiac(_) => ZodiacKind::shop_price(),
        Consumable::Talisman(t) => t.shop_price(),
    };
    (buy / 2).max(1)
}

fn owned_consumable_count(run: &crate::game::run::RunState, pred: fn(Consumable) -> bool) -> usize {
    run.consumables
        .items
        .iter()
        .copied()
        .filter(|&c| pred(c))
        .count()
}

fn live_shop_hit(
    hit: ShopHit,
    items: &[ShopItem],
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    pack_item: &Option<TilePackShopItem>,
    run: &crate::game::run::RunState,
) -> Option<ShopHit> {
    let valid = match hit {
        ShopHit::Relic(i) => i < items.len() + run.relics.active.len(),
        ShopHit::Ribbon(i) => {
            i < zodiac_items.len()
                + owned_consumable_count(run, |c| matches!(c, Consumable::Zodiac(_)))
        }
        ShopHit::Talisman(i) => {
            i < talisman_items.len()
                + owned_consumable_count(run, |c| matches!(c, Consumable::Talisman(_)))
        }
        ShopHit::Dish(id) => {
            matches!(
                id,
                PICK_RELIC_DISH | PICK_COIN_DISH | PICK_JOURNAL_BOOK | PICK_TILE_PACK
            ) && (id != PICK_TILE_PACK || pack_item.is_some())
        }
        ShopHit::TilePack(id) => id == PICK_TILE_PACK && pack_item.is_some(),
    };
    valid.then_some(hit)
}

fn owned_ribbon_inventory_index(
    ribbon_idx: usize,
    zodiac_items: &[ConsumableShopItem],
    run: &crate::game::run::RunState,
) -> Option<usize> {
    if ribbon_idx < zodiac_items.len() {
        return None;
    }
    let oi = ribbon_idx - zodiac_items.len();
    let mut count = 0usize;
    for (idx, c) in run.consumables.items.iter().enumerate() {
        if matches!(c, Consumable::Zodiac(_)) {
            if count == oi {
                return Some(idx);
            }
            count += 1;
        }
    }
    None
}

fn owned_talisman_inventory_index(
    talisman_idx: usize,
    talisman_items: &[ConsumableShopItem],
    run: &crate::game::run::RunState,
) -> Option<usize> {
    if talisman_idx < talisman_items.len() {
        return None;
    }
    let oi = talisman_idx - talisman_items.len();
    let mut count = 0usize;
    for (idx, c) in run.consumables.items.iter().enumerate() {
        if matches!(c, Consumable::Talisman(_)) {
            if count == oi {
                return Some(idx);
            }
            count += 1;
        }
    }
    None
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

impl ShopItem {
    fn buy_label(&self) -> String {
        if self.price == 0 {
            "FREE".to_string()
        } else {
            format!("Buy {}g", self.price)
        }
    }
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
            Consumable::Talisman(t) => t.shop_price(),
        }
    }
    fn name(&self) -> String {
        self.consumable.name()
    }
    fn description(&self) -> String {
        match self.consumable {
            Consumable::Zodiac(z) => {
                let yk = z.yaku();
                format!(
                    "Levels {} for the rest of the run (+0.5 mult, +20 chips per level). {}",
                    yk.name(),
                    super::meld_guide::yaku_shape_text(yk),
                )
            }
            Consumable::Talisman(t) => t.description().to_string(),
        }
    }
}

/// A purchasable tile pack in the shop.
struct TilePackShopItem {
    kind: TilePackKind,
    sold: bool,
}

fn push_free_badge(
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
    rect: [f32; 4],
    window_h: f32,
    ui_scale: f32,
) {
    if rect[2] <= 1.0 || rect[3] <= 1.0 || !rect[0].is_finite() || !rect[1].is_finite() {
        return;
    }
    let badge_font = typography::size(typography::MICRO, window_h, ui_scale).max(12.0);
    let badge_h = (badge_font * 1.55).max(18.0);
    let badge_w = (badge_font * 4.8).max(52.0);
    let badge_x = rect[0] + rect[2] - badge_w * 0.88;
    let badge_y = rect[1] - badge_h * 0.22;

    quads.push(GpuInstance {
        rect: [badge_x, badge_y, badge_w, badge_h],
        color: [color::BRASS[0], color::BRASS[1], color::BRASS[2], 0.95],
    });
    quads.push(GpuInstance {
        rect: [badge_x + 2.0, badge_y + 2.0, badge_w - 4.0, badge_h - 4.0],
        color: [
            color::MIDNIGHT[0],
            color::MIDNIGHT[1],
            color::MIDNIGHT[2],
            0.96,
        ],
    });
    texts.push(TextLabel {
        rect: [badge_x + 4.0, badge_y, badge_w - 8.0, badge_h],
        text: "FREE".to_string(),
        color: color::CHAMPAGNE,
        font_px: Some(badge_font),
        align: TextAlign::Center,
        ..Default::default()
    });
}

/// Which phase the pack celebration is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CelebPhase {
    /// Show the pack box up close; wait for player click to open.
    Closeup,
    /// Tiles flying out and arranging in a row.
    Reveal,
}

/// Tile-pack opening celebration overlay. Blocks shop input while active
/// and shows the acquired tiles flying out of the pack one by one, then
/// arranging themselves in a readable row.
struct PackCelebration {
    /// The tiles acquired from this pack (with enhancements already stamped).
    tiles: Vec<Tile>,
    /// Pack name for the header label.
    pack_name: &'static str,
    /// Which pack kind this is (for rendering the box texture).
    pack_kind: TilePackKind,
    /// Current phase of the celebration.
    phase: CelebPhase,
    /// Wall-clock time the *reveal* phase started (after closeup).
    started_at: Instant,
    /// Whether the player has dismissed the celebration (confirm/click).
    dismissed: bool,
    /// Number of tiles whose reveal sound has already been fired.
    revealed_count: usize,
}

impl PackCelebration {
    /// Total seconds for all tiles to have arrived in their final positions.
    /// Each tile gets a staggered start: tile *i* begins at `i * STAGGER`.
    const STAGGER: f32 = 0.18;
    /// Duration of a single tile's flight from the pack to its slot.
    const TILE_FLY_SECS: f32 = 0.35;
    /// Extra pause after the last tile lands before the "press to continue"
    /// prompt appears.
    const SETTLE_SECS: f32 = 0.20;

    fn new(tiles: Vec<Tile>, pack_name: &'static str, pack_kind: TilePackKind) -> Self {
        Self {
            tiles,
            pack_name,
            pack_kind,
            phase: CelebPhase::Closeup,
            // Will be reset when transitioning to Reveal.
            started_at: Instant::now(),
            dismissed: false,
            revealed_count: 0,
        }
    }

    /// Total animation duration (all tiles landed + settle pause).
    fn total_duration(&self) -> f32 {
        let n = self.tiles.len().max(1) as f32;
        (n - 1.0) * Self::STAGGER + Self::TILE_FLY_SECS + Self::SETTLE_SECS
    }

    /// Elapsed seconds since the celebration started.
    fn elapsed(&self) -> f32 {
        Instant::now()
            .saturating_duration_since(self.started_at)
            .as_secs_f32()
    }

    /// True once all tiles have landed and the settle pause is over.
    fn fully_settled(&self) -> bool {
        self.elapsed() >= self.total_duration()
    }

    /// Per-tile animation progress: 0.0 = hasn't started, 1.0 = landed.
    fn tile_progress(&self, idx: usize) -> f32 {
        let t = self.elapsed() - idx as f32 * Self::STAGGER;
        (t / Self::TILE_FLY_SECS).clamp(0.0, 1.0)
    }
}

/// Zodiac ribbon close-up celebration: shows the purchased ribbon front
/// and center with a sinusoidal snake/sway animation before dismissing
/// into the yaku level-up popup.
struct ZodiacCelebration {
    kind: ZodiacKind,
    yaku_name: &'static str,
    new_level: u32,
    started_at: Instant,
    dismissed: bool,
}

impl ZodiacCelebration {
    /// Total close-up duration before auto-dismissing (seconds).
    const DURATION: f32 = 2.0;

    fn new(kind: ZodiacKind, yaku_name: &'static str, new_level: u32) -> Self {
        Self {
            kind,
            yaku_name,
            new_level,
            started_at: Instant::now(),
            dismissed: false,
        }
    }

    fn elapsed(&self) -> f32 {
        Instant::now()
            .saturating_duration_since(self.started_at)
            .as_secs_f32()
    }

    fn is_done(&self) -> bool {
        self.dismissed || self.elapsed() >= Self::DURATION
    }
}

pub struct ShopScene {
    pub came_from_round: u32,
    mode: ShopMode,
    items: Vec<ShopItem>,
    zodiac_items: Vec<ConsumableShopItem>,
    talisman_items: Vec<ConsumableShopItem>,
    pack_item: Option<TilePackShopItem>,
    /// Current reroll cost — starts at `REROLL_BASE_COST` and increases by
    /// `REROLL_COST_INCREMENT` each time the player rerolls this shop visit.
    reroll_cost: u32,
    pause_menu: PauseMenu,
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
    /// Active tile-pack opening celebration, if any.
    pack_celebration: Option<PackCelebration>,
    /// Active zodiac ribbon close-up celebration, if any.
    zodiac_celebration: Option<ZodiacCelebration>,
    /// Floating 3D text popups for zodiac level-up feedback.
    score_popups: ScorePopupSystem,
    /// Particle burst effects for zodiac level-up feedback.
    particles: ParticleSystem,
    /// Timestamp of the previous frame — used to compute `dt` for
    /// particle and popup animation.
    last_frame: Instant,
    /// Monotonic accumulated time in seconds — drives idle coin/bar
    /// animation in `draw_frame` without calling `Instant::now()`.
    age_secs: f32,
    /// Per-relic glow start times. Populated when `relic_activations` is
    /// drained from the run state (e.g. Bonfire on relic sell, RitualBlade
    /// on destroy). Drives glow + wiggle on owned relics in the shop.
    relic_glow_starts: std::collections::HashMap<RelicId, Instant>,
}

/// Click id for the `?` glossary badge in the shop HUD.
const SHOP_HELP_BADGE_ID: u32 = 0x9100;
/// Click id for the catch-all 3D-hit dispatcher. When clicked, the shop's
/// update() routes the click based on `UpdateCtx::picked_shop_object`.
const SHOP_3D_HIT_ID: u32 = 0x9200;
/// Click id for the Next Round 2D button.
const SHOP_NEXT_ROUND_ID: u32 = 0x9300;
/// Click id for the Reroll 2D button.
const SHOP_REROLL_ID: u32 = 0x9400;
/// Floating sell button for hovered owned relics.
const SHOP_SELL_RELIC_BASE: u32 = 0x9500;
/// Floating sell button for hovered owned consumables.
const SHOP_SELL_CONSUMABLE_BASE: u32 = 0x9600;
/// Base gold cost for the first shop reroll.
const REROLL_BASE_COST: u32 = 5;
/// How long a relic glow + wiggle lasts after activation.
const RELIC_GLOW_LIFETIME: std::time::Duration = std::time::Duration::from_millis(900);
/// How much the reroll cost increases per use within a single shop visit.
const REROLL_COST_INCREMENT: u32 = 5;
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
/// Pick id for the for-sale tile pack on the shop shelf.
const PICK_TILE_PACK: u32 = 4;

/// Generate randomized shop stock (relics + consumables) from the player's
/// unowned-relic pool. Shared between initial shop creation and rerolls.
fn generate_shop_stock(
    relics: &RelicState,
    extra_relics: usize,
) -> (
    Vec<ShopItem>,
    Vec<ConsumableShopItem>,
    Vec<ConsumableShopItem>,
    Option<TilePackShopItem>,
) {
    let mut rng = rand::rng();

    const MAX_RIBBONS: usize = 4;
    let max_relics = NICHE_COLS * NICHE_ROWS;

    let mut n_relics = rng.random_range(0..=max_relics) + extra_relics;
    let mut n_zodiacs = rng.random_range(1..=MAX_RIBBONS);
    let mut n_talismans = rng.random_range(1..=MAX_RIBBONS);
    if n_zodiacs + n_talismans > MAX_RIBBONS {
        while n_zodiacs + n_talismans > MAX_RIBBONS {
            if n_talismans >= n_zodiacs {
                n_talismans -= 1;
            } else {
                n_zodiacs -= 1;
            }
        }
    }
    while n_relics + n_zodiacs + n_talismans < 2 {
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

    let defs = all_relic_defs();
    // Some relics are never offered in the shop — they only appear via
    // special means (transformation, duplication, etc.).
    let shop_excluded = [RelicId::IronLantern, RelicId::PhantomRelic];
    let mut relic_pool: Vec<&_> = defs
        .iter()
        .filter(|d| !relics.owns(d.id) && !shop_excluded.contains(&d.id))
        .collect();
    relic_pool.shuffle(&mut rng);
    let items: Vec<ShopItem> = relic_pool
        .into_iter()
        .take(n_relics)
        .map(|d| ShopItem {
            relic: d.id,
            name: d.name,
            description: d.description,
            rarity: d.rarity,
            price: relic_shop_price(d.id, relics),
            sold: false,
        })
        .collect();

    let mut zodiac_pool: Vec<ZodiacKind> = ZodiacKind::all().iter().copied().collect();
    zodiac_pool.shuffle(&mut rng);
    let mut talisman_pool: Vec<TalismanKind> = TalismanKind::all().iter().copied().collect();
    talisman_pool.shuffle(&mut rng);
    let zodiac_items: Vec<ConsumableShopItem> = zodiac_pool
        .into_iter()
        .take(n_zodiacs)
        .map(|z| ConsumableShopItem {
            consumable: Consumable::Zodiac(z),
            sold: false,
        })
        .collect();
    let talisman_items: Vec<ConsumableShopItem> = talisman_pool
        .into_iter()
        .take(n_talismans)
        .map(|t| ConsumableShopItem {
            consumable: Consumable::Talisman(t),
            sold: false,
        })
        .collect();

    // Always offer a tile pack.
    let pack_item = {
        let mut pack_pool: Vec<TilePackKind> = TilePackKind::all().to_vec();
        pack_pool.shuffle(&mut rng);
        pack_pool
            .first()
            .map(|&kind| TilePackShopItem { kind, sold: false })
    };

    (items, zodiac_items, talisman_items, pack_item)
}

fn tutorial_shop_stock() -> (
    Vec<ShopItem>,
    Vec<ConsumableShopItem>,
    Vec<ConsumableShopItem>,
    Option<TilePackShopItem>,
) {
    let defs = all_relic_defs();
    let pair_power = defs
        .iter()
        .find(|d| d.id == RelicId::PairPower)
        .expect("Pair Power relic def should exist");
    (
        vec![ShopItem {
            relic: pair_power.id,
            name: pair_power.name,
            description: pair_power.description,
            rarity: pair_power.rarity,
            price: relic_shop_price(pair_power.id, &RelicState::default()),
            sold: false,
        }],
        vec![ConsumableShopItem {
            consumable: Consumable::Zodiac(ZodiacKind::Dragon),
            sold: false,
        }],
        vec![ConsumableShopItem {
            consumable: Consumable::Talisman(TalismanKind::Jade),
            sold: false,
        }],
        Some(TilePackShopItem {
            kind: TilePackKind::ScrollLibrary,
            sold: false,
        }),
    )
}

impl ShopScene {
    pub fn new(came_from_round: u32, run: &mut crate::game::run::RunState) -> Self {
        Self::new_with_mode(came_from_round, run, ShopMode::Standard)
    }

    pub fn new_tutorial(run: &mut crate::game::run::RunState) -> Self {
        Self::new_with_mode(run.run_number, run, ShopMode::Tutorial)
    }

    fn new_with_mode(
        came_from_round: u32,
        run: &mut crate::game::run::RunState,
        mode: ShopMode,
    ) -> Self {
        let extra_relics: usize = if run.tag_rich_stock { 2 } else { 0 };
        let extra_relics = if run.tag_patron_gift {
            extra_relics.max(1)
        } else {
            extra_relics
        };
        let (mut items, zodiac_items, talisman_items, pack_item) = if mode == ShopMode::Tutorial {
            tutorial_shop_stock()
        } else {
            generate_shop_stock(&run.relics, extra_relics)
        };

        // PatronGift: zero out one random relic's price.
        if mode == ShopMode::Standard && run.tag_patron_gift && !items.is_empty() {
            use rand::prelude::IndexedMutRandom;
            let mut rng = rand::rng();
            if let Some(item) = items.choose_mut(&mut rng) {
                item.price = 0;
            }
        }

        let reroll_cost = if mode == ShopMode::Tutorial {
            u32::MAX
        } else if run.tag_free_reroll {
            0
        } else {
            REROLL_BASE_COST
        };

        // Clear consumed tag flags.
        run.tag_free_reroll = false;
        run.tag_patron_gift = false;
        run.tag_rich_stock = false;

        Self {
            came_from_round,
            mode,
            items,
            zodiac_items,
            talisman_items,
            pack_item,
            reroll_cost,
            pause_menu: PauseMenu::new(),
            journal: super::journal::JournalOverlay::new(),
            focus: None,
            last_focus_rects: std::cell::RefCell::new(Vec::new()),
            pack_celebration: None,
            zodiac_celebration: None,
            score_popups: ScorePopupSystem::new(),
            particles: ParticleSystem::new(),
            last_frame: Instant::now(),
            age_secs: 0.0,
            relic_glow_starts: std::collections::HashMap::new(),
        }
    }

    fn continue_scene(&self, run: &mut crate::game::run::RunState) -> Scene {
        if self.mode == ShopMode::Tutorial {
            if let Some(ref mut onboarding) = run.onboarding {
                onboarding.phase = OnboardingPhase::Finale;
            }
            run.begin_onboarding_finale();
            Scene::Gameplay(super::gameplay::GameplayScene::new())
        } else {
            Scene::PickBlind(PickBlindScene::new())
        }
    }

    /// Route a `ShopActionResult` to the appropriate visual feedback.
    fn handle_shop_action_result(
        &mut self,
        result: ShopActionResult,
        _cursor_pos: (f32, f32),
        bus: &mut crate::game::event_bus::EventBus,
    ) {
        match result {
            ShopActionResult::None => {}
            ShopActionResult::PackCelebration(celeb) => {
                self.pack_celebration = Some(celeb);
            }
            ShopActionResult::ZodiacApplied {
                zodiac_kind,
                yaku_name,
                new_level,
            } => {
                self.zodiac_celebration =
                    Some(ZodiacCelebration::new(zodiac_kind, yaku_name, new_level));
                bus.push(crate::game::event_bus::GameEvent::ZodiacReveal);
            }
        }
    }

    /// Spawn the yaku level-up popup + particle burst at the center of the
    /// screen. Called when a zodiac celebration finishes (either by timeout
    /// or player dismiss).
    fn finish_zodiac_celebration(
        &mut self,
        w: f32,
        h: f32,
        bus: &mut crate::game::event_bus::EventBus,
    ) {
        if let Some(celeb) = self.zodiac_celebration.take() {
            bus.push(crate::game::event_bus::GameEvent::ZodiacLevelUp);
            let label = format!("{} Lvl.{}", celeb.yaku_name, celeb.new_level);
            let center = (w * 0.5, h * 0.45);
            let dest = (center.0, center.1 - 200.0);
            self.score_popups.spawn(
                label,
                center,
                dest,
                crate::core::scoring::StepKind::Gold,
                celeb.new_level as f32,
            );
            self.particles
                .emit(center.0, center.1, 24, [0.95, 0.78, 0.25, 1.0], 0.9);
        }
    }

    /// Replace all unsold stock with fresh random items and bump the cost.
    fn reroll(&mut self, run: &mut crate::game::run::RunState) {
        if self.mode == ShopMode::Tutorial {
            return;
        }
        run.gold -= self.reroll_cost as i32;
        self.reroll_cost += REROLL_COST_INCREMENT;
        let (items, zodiac_items, talisman_items, pack_item) = generate_shop_stock(&run.relics, 0);
        self.items = items;
        self.zodiac_items = zodiac_items;
        self.talisman_items = talisman_items;
        self.pack_item = pack_item;
        self.focus = None;
    }

    /// Debug-only: reroll stock without deducting gold or incrementing cost.
    pub fn debug_reroll(&mut self, run: &crate::game::run::RunState) {
        let (items, zodiac_items, talisman_items, pack_item) = generate_shop_stock(&run.relics, 0);
        self.items = items;
        self.zodiac_items = zodiac_items;
        self.talisman_items = talisman_items;
        self.pack_item = pack_item;
        self.focus = None;
    }

    /// Debug-only: open a random tile pack celebration without purchasing.
    pub fn debug_open_pack(&mut self, run: &mut crate::game::run::RunState) {
        use crate::core::tile_pack::{PACK_ID_STRIDE, PACK_TILE_ID_BASE, TilePackKind};
        let mut rng = rand::rng();
        let all = TilePackKind::all();
        let kind = all[rand::RngExt::random_range(&mut rng, 0..all.len())];
        let pack_idx = run.tile_packs.len();
        let start_id = PACK_TILE_ID_BASE + (pack_idx as u32) * PACK_ID_STRIDE;
        let mut tiles = kind.generate_tiles(start_id);
        if let Some(enh) = kind.pre_enhancement() {
            for t in &mut tiles {
                run.tile_enhancements.insert(t.id, enh);
                t.enhancement = Some(enh);
            }
        }
        run.tile_packs.push(kind);
        self.pack_celebration = Some(PackCelebration::new(tiles, kind.name(), kind));
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
    // ── For-sale zodiac ribbon anchors (upper-right cabinet zone) ──
    ribbon_anchors_px: [(f32, f32, f32); 8],
    ribbon_count: usize,
    ribbon_length: f32,
    ribbon_width: f32,
    // ── For-sale talisman anchors (lower-right cabinet zone, below shelf divider) ──
    talisman_anchors_px: [(f32, f32, f32); 4],
    talisman_anchor_count: usize,
    talisman_wall_width: f32,
    // ── Foreground dishes ──
    relic_dish_center_px: (f32, f32, f32),
    relic_dish_extents: [f32; 3],
    coin_dish_center_px: (f32, f32, f32),
    coin_dish_extents: [f32; 3],
    // ── Owned-relic positions inside the relic dish ──
    owned_relic_count: usize,
    // ── For-sale tile pack (in cabinet, below ribbons) ──
    pack_center_px: (f32, f32, f32),
    pack_extents: [f32; 3],
    // ── Owned consumable row (shared tray on the inventory shelf) ──
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
        n_for_sale_zodiacs: usize,
        n_for_sale_talismans: usize,
        n_owned_relics: usize,
        n_owned_consumables: usize,
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
        let n_ribbons = n_for_sale_zodiacs.min(8);
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
        // Hanging zodiac ribbons — sized layout-relative (not mm) so they
        // hold their visual weight inside the (also layout-relative) cabinet
        // at every resolution.
        let ribbon_width = h * 0.085;
        let ribbon_length = ribbon_width * 1.5;

        // ── For-sale talisman anchors: below the shelf divider ──────────
        // The cabinet mesh has a physical shelf at RIGHT_SHELF_Y that
        // separates the upper zodiac zone from the lower talisman zone.
        // Talismans are spread horizontally across the same right-side
        // area as zodiacs, but pinned below the shelf.
        let mut talisman_anchors_px: [(f32, f32, f32); 4] = [(0.0, 0.0, 0.0); 4];
        let n_talisman_anchors = n_for_sale_talismans.min(4);
        let talisman_wall_width = ribbon_width;
        if n_talisman_anchors > 0 {
            let right_lx0 = crate::render::curio_cabinet_mesh::DIVIDER_X + 0.0125;
            let right_lx1 = 0.5 - 0.04;
            let right_inner_w = right_lx1 - right_lx0;
            let right_x0 =
                cabinet_pixel_x + (right_lx0 + right_inner_w * 0.25) * cabinet_extents[0];
            let right_x1 =
                cabinet_pixel_x + (right_lx0 + right_inner_w * 0.78) * cabinet_extents[0];
            let avail = right_x1 - right_x0;
            let step = if n_talisman_anchors > 1 {
                avail / (n_talisman_anchors as f32 - 1.0)
            } else {
                0.0
            };
            // Pin below the shelf divider, centered in the lower zone.
            let talisman_pin_world_y = cabinet_world_y
                + (crate::render::curio_cabinet_mesh::RIGHT_SHELF_Y - 0.14) * cabinet_extents[1];
            let talisman_pin_pixel_y = cabinet_front_py + cabinet_extents[2] * 0.05;
            for i in 0..n_talisman_anchors {
                let x = if n_talisman_anchors == 1 {
                    (right_x0 + right_x1) * 0.5
                } else {
                    right_x0 + step * i as f32
                };
                talisman_anchors_px[i] = (x, talisman_pin_pixel_y, talisman_pin_world_y);
            }
        }

        // ── For-sale tile pack: right side of the cabinet, below ribbons ─
        // Centered horizontally in the ribbon area, sitting on the lower
        // shelf of the cabinet so it reads as a boxed product on display.
        let right_lx0 = crate::render::curio_cabinet_mesh::DIVIDER_X + 0.0125;
        let right_lx1 = 0.5 - 0.04;
        let pack_local_x = (right_lx0 + right_lx1) * 0.5;
        let pack_px = cabinet_pixel_x + pack_local_x * cabinet_extents[0];
        // Push the pack against the back wall so it leans naturally.
        let pack_py = cabinet_front_py - cabinet_extents[2] * 0.60;
        let pack_wy = cabinet_world_y - cabinet_extents[1] * 0.20;
        let pack_center_px = (pack_px, pack_py, pack_wy);
        // Pack proportions: wide front face (x × y), thin depth (z).
        // Like a real trading card pack displayed the long way.
        let pack_extents = [
            cabinet_extents[0] * 0.07,
            cabinet_extents[1] * 0.18,
            cabinet_extents[0] * 0.02,
        ];

        // ── Inventory shelf (bottom band) ────────────────────────────────
        // The bottom ~25% of the screen is the player's inventory shelf:
        // a single tray broken into three sub-zones — owned relics on the
        // left (widest), owned consumables center-right, coin dish on the
        // far right. Symmetry of language: relics and consumables share
        // the same "laid flat in a tray" treatment so they parse as "your
        // collection" rather than two unrelated systems.
        // Inventory shelf. Raised to h*0.62 so the relic and consumable
        // trays sit higher — closer to the cabinet and further from the
        // bottom-of-screen buttons.
        let shelf_y = h * 0.62;
        // All three inventory trays share a ~12mm brass rim — small
        // decorative dishes that hold the player's items. Footprint
        // (width/depth) stays layout-relative so the shelf reads as a
        // wide band across the bottom of the screen at any resolution.
        let dish_rim = layout.mm(12.0);
        // Relic tray spans 16%–44% w → center at 30% w, half-width 14% w.
        let relic_dish_center_px = (w * 0.30, shelf_y, 0.0);
        let relic_dish_extents = [w * 0.14, dish_rim, h * 0.10];
        // Consumable tray: owned zodiac ribbons + talisman tablets laid flat.
        let consumable_row_center_px = (w * 0.58, shelf_y, 0.0);
        let consumable_row_extents = [w * 0.10, dish_rim, h * 0.10];
        // Coin dish — sits just right of the consumable tray, pulled
        // forward toward the camera so the gold reads larger on screen.
        let coin_dish_center_px = (w * 0.62, h * 1.1, h * 0.3);
        let coin_dish_extents = [w * 0.045, dish_rim, h * 0.07];

        // ── Owned consumable sizing ─────────────────────────────────────
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
            talisman_anchors_px,
            talisman_anchor_count: n_talisman_anchors,
            talisman_wall_width,
            pack_center_px,
            pack_extents,
            relic_dish_center_px,
            relic_dish_extents,
            coin_dish_center_px,
            coin_dish_extents,
            owned_relic_count: n_owned_relics,
            consumable_row_center_px,
            consumable_row_extents,
            consumable_count: n_owned_consumables,
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
        let wy = self.relic_dish_extents[1] + 4.0;
        (px, py, wy)
    }

    /// Center pixel coords + base world_y of the i-th owned consumable
    /// laid flat in the consumable tray. Single row, evenly spaced.
    fn consumable_pos(&self, idx: usize) -> (f32, f32, f32) {
        let n = self.consumable_count.max(1) as f32;
        let row_w = self.consumable_row_extents[0] * 0.85;
        let start_x = self.consumable_row_center_px.0 - row_w * 0.5 + (row_w / n) * 0.5;
        let px = start_x + (row_w / n) * idx as f32;
        let py = self.consumable_row_center_px.1;
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
    let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
    // Front face (x × y) is square to match the 1:1 relic textures.
    let face = base * (0.65 + r0 * 0.45);
    [face, face, base * (0.55 + r2 * 0.40)]
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
            TalismanKind::Kiln => [0.85, 0.35, 0.18, 1.0], // kiln orange-red
        },
    }
}

/// Tiered gold display: coin strings for small amounts, gold bars for large.
///
/// **Denomination breakdown** (applied when gold ≥ 100):
///   big bars   = gold / 100       (each worth 100)
///   mini bars  = (gold % 100) / 25 (each worth 25)
///   coins      = gold % 25        (remainder as vertical strings of 10)
///
/// Below 100 gold the display is pure coin strings (10 per string).
fn coin_display_layout(
    gold: u32,
    dish_center_px: (f32, f32, f32),
    dish_extents: [f32; 3],
    time: f32,
) -> (Vec<GoldBarPlacement>, Vec<CoinPlacement>) {
    if gold == 0 {
        return (Vec::new(), Vec::new());
    }

    let coin_radius = 9.0_f32;
    let coin_thickness = 3.5_f32;
    let coins_per_string: u32 = 10;
    let dish_top_y = dish_center_px.2 + dish_extents[1] + 2.0;
    let gold_color: [f32; 4] = [1.00, 0.78, 0.30, 1.0];
    // Slightly darker gold for bars so they read as a distinct denomination.
    let bar_color: [f32; 4] = [0.92, 0.72, 0.22, 1.0];

    // ── Denomination split ────────────────────────────────────────────
    let big_bars = gold / 100;
    let after_big = gold % 100;
    let mini_bars = after_big / 25;
    let coin_gold = after_big % 25;

    let total_bars = (big_bars + mini_bars) as usize;
    let n_coin_strings = if coin_gold > 0 {
        ((coin_gold - 1) / coins_per_string + 1) as usize
    } else {
        0
    };
    let total_coins = (coin_gold as usize).min(crate::render::wgpu_renderer::MAX_COIN_SLOTS);

    // ── Spatial budget ────────────────────────────────────────────────
    // Bars sit in a row at the back of the dish; coin strings sit in
    // front. Both are centered within the dish footprint.
    let footprint_x = dish_extents[0] * 0.90;

    // ── Bars ──────────────────────────────────────────────────────────
    let mut bars = Vec::with_capacity(total_bars);
    if total_bars > 0 {
        let big_he: [f32; 3] = [7.0, 4.0, 5.0];
        let mini_he: [f32; 3] = [5.0, 3.0, 3.5];
        // Lay bars in a single row; if more than fit side-by-side, stack
        // additional layers on top.
        let max_per_row = ((footprint_x * 2.0) / (big_he[0] * 2.5)).floor().max(1.0) as usize;
        let mut bar_idx: usize = 0;
        // Big bars first, then mini bars.
        let bar_specs: Vec<(usize, [f32; 3])> = std::iter::repeat((0, big_he))
            .take(big_bars as usize)
            .chain(std::iter::repeat((1, mini_he)).take(mini_bars as usize))
            .collect();
        for (spec_i, (_kind, he)) in bar_specs.iter().enumerate() {
            let row = bar_idx / max_per_row;
            let col = bar_idx % max_per_row;
            let cols_this_row = max_per_row.min(total_bars - row * max_per_row);
            let row_width = cols_this_row as f32 * he[0] * 2.5;
            let x_off = -row_width * 0.5 + he[0] * 1.25 + col as f32 * he[0] * 2.5;
            let world_y = dish_top_y + he[1] + row as f32 * (big_he[1] * 2.0 + 1.0);
            // Bars sit toward the back of the dish.
            let z_off = -dish_extents[2] * 0.25;
            // Gentle rotation drift for sparkle.
            let rot = 0.02 * (time * 0.5 + spec_i as f32 * 2.3).sin();
            bars.push(GoldBarPlacement {
                world_pos: [dish_center_px.0 + x_off, dish_center_px.1 + z_off, world_y],
                rotation_y: rot,
                half_extents: *he,
                color: bar_color,
            });
            bar_idx += 1;
        }
    }

    // ── Coin strings ─────────────────────────────────────────────────
    let mut coins = Vec::with_capacity(total_coins);
    if total_coins > 0 {
        // Strings sit in a row toward the front of the dish. When bars
        // are present they shift forward; when alone they center in the
        // dish.
        let z_off = if total_bars > 0 {
            dish_extents[2] * 0.25
        } else {
            0.0
        };
        let string_spacing = (coin_radius * 2.2).min(if n_coin_strings > 1 {
            (footprint_x * 2.0) / (n_coin_strings as f32)
        } else {
            coin_radius * 2.2
        });
        let row_width = n_coin_strings as f32 * string_spacing;
        let mut placed = 0u32;
        for s in 0..n_coin_strings {
            let x_off = -row_width * 0.5 + string_spacing * 0.5 + s as f32 * string_spacing;
            let coins_in_this_string = coins_per_string.min(coin_gold - placed);
            for c in 0..coins_in_this_string {
                let si = s as f32;
                let ci = c as f32;
                // Idle sway + bob.
                let sway = 0.04 * (time * 1.2 + si * 1.8).sin();
                let bob = 0.3 * (time * 0.9 + si * 2.1 + ci * 0.4).sin();
                let base_rot = si * 0.15;
                let world_y = dish_top_y + ci * coin_thickness + bob;
                coins.push(CoinPlacement {
                    world_pos: [dish_center_px.0 + x_off, dish_center_px.1 + z_off, world_y],
                    rotation_y: base_rot + sway,
                    radius: coin_radius,
                    thickness: coin_thickness,
                    color: gold_color,
                });
                placed += 1;
            }
        }
    }

    (bars, coins)
}

impl SceneBehavior for ShopScene {
    fn pause_options_overlay(&self) -> Option<&super::options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    fn has_blocking_overlay(&self) -> bool {
        self.pause_menu.paused
            || self.journal.open
            || self.pack_celebration.is_some()
            || self.zodiac_celebration.is_some()
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.age_secs += dt;
        self.particles.update(dt);
        self.score_popups.update(now);

        // Drain relic activations and evict expired glows.
        for rid in ctx.run.relic_activations.drain(..) {
            self.relic_glow_starts.insert(rid, now);
            ctx.bus
                .push(crate::game::event_bus::GameEvent::RelicActivated(rid));
        }
        self.relic_glow_starts
            .retain(|_, start| now.saturating_duration_since(*start) < RELIC_GLOW_LIFETIME);

        // Help action opens the Meld Guide scene.
        for &cid in ctx.button_clicks {
            if cid == SHOP_HELP_BADGE_ID {
                return Some(Scene::MeldGuide(super::meld_guide::MeldGuideScene::new(
                    true,
                )));
            }
        }
        for a in ctx.actions {
            if matches!(a, UiAction::Help) {
                return Some(Scene::MeldGuide(super::meld_guide::MeldGuideScene::new(
                    true,
                )));
            }
        }

        // Yaku Journal overlay — opened by clicking the Journal book on
        // the counter (routed via the 3D-hit dispatcher below as
        // `ShopHit::Dish(PICK_JOURNAL_BOOK)`).
        if self.journal.open {
            self.journal
                .handle_input(ctx.actions, ctx.button_clicks, ctx.scroll_lines);
            return None;
        }

        // Tile-pack opening celebration — swallow all input until
        // the player dismisses it with Confirm or any click.
        if let Some(ref mut celeb) = self.pack_celebration {
            let has_input = ctx.actions.iter().any(|a| {
                matches!(
                    a,
                    UiAction::Confirm | UiAction::Cancel | UiAction::CommitDiscard
                )
            }) || !ctx.button_clicks.is_empty();

            match celeb.phase {
                CelebPhase::Closeup => {
                    // Wait for player click/confirm to tear open the pack.
                    if has_input {
                        celeb.phase = CelebPhase::Reveal;
                        celeb.started_at = Instant::now();
                        ctx.bus.push(crate::game::event_bus::GameEvent::PackOpened);
                    }
                }
                CelebPhase::Reveal => {
                    // Fire a sound event for each newly-revealed tile.
                    let n = celeb.tiles.len();
                    while celeb.revealed_count < n
                        && celeb.tile_progress(celeb.revealed_count) > 0.0
                    {
                        ctx.bus
                            .push(crate::game::event_bus::GameEvent::PackTileRevealed);
                        celeb.revealed_count += 1;
                    }
                    let dominated = celeb.fully_settled() || celeb.dismissed;
                    if dominated && has_input {
                        self.pack_celebration = None;
                        return None;
                    }
                }
            }
            return None;
        }

        // Zodiac ribbon close-up celebration — swallow input until
        // the player dismisses or the timer expires.
        if let Some(ref mut celeb) = self.zodiac_celebration {
            let has_input = ctx.actions.iter().any(|a| {
                matches!(
                    a,
                    UiAction::Confirm | UiAction::Cancel | UiAction::CommitDiscard
                )
            }) || !ctx.button_clicks.is_empty();
            if has_input {
                celeb.dismissed = true;
            }
            if celeb.is_done() {
                let w = ctx.layout.window_w;
                let h = ctx.layout.window_h;
                self.finish_zodiac_celebration(w, h, ctx.bus);
            }
            return None;
        }

        // Pause menu handling.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            // Drain a meld guide request from the pause menu.
            if self.pause_menu.take_meld_guide_request() {
                return Some(Scene::MeldGuide(super::meld_guide::MeldGuideScene::new(
                    true,
                )));
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
            // LB / `[` sells the focused owned relic or consumable. RB keeps
            // the legacy relic reordering affordance for Mirror Tile setups.
            if matches!(a, UiAction::NavigateHudNext | UiAction::NavigateHudPrev) {
                if matches!(a, UiAction::NavigateHudPrev) {
                    match self.focus {
                        Some(ShopFocus::Relic(i)) => {
                            let n_for_sale = self.items.len();
                            if i >= n_for_sale {
                                let owned_idx = i - n_for_sale;
                                let result = apply_shop_action(
                                    ShopAction::SellRelic(owned_idx),
                                    &mut self.items,
                                    &mut self.zodiac_items,
                                    &mut self.talisman_items,
                                    &mut self.pack_item,
                                    ctx.run,
                                    ctx.bus,
                                );
                                self.handle_shop_action_result(result, ctx.cursor_pos, ctx.bus);
                                self.focus = None;
                            }
                            continue;
                        }
                        Some(ShopFocus::Ribbon(i)) => {
                            if let Some(inv_idx) =
                                owned_ribbon_inventory_index(i, &self.zodiac_items, ctx.run)
                            {
                                let result = apply_shop_action(
                                    ShopAction::SellConsumable(inv_idx),
                                    &mut self.items,
                                    &mut self.zodiac_items,
                                    &mut self.talisman_items,
                                    &mut self.pack_item,
                                    ctx.run,
                                    ctx.bus,
                                );
                                self.handle_shop_action_result(result, ctx.cursor_pos, ctx.bus);
                                self.focus = None;
                            }
                            continue;
                        }
                        Some(ShopFocus::Talisman(i)) => {
                            if let Some(inv_idx) =
                                owned_talisman_inventory_index(i, &self.talisman_items, ctx.run)
                            {
                                let result = apply_shop_action(
                                    ShopAction::SellConsumable(inv_idx),
                                    &mut self.items,
                                    &mut self.zodiac_items,
                                    &mut self.talisman_items,
                                    &mut self.pack_item,
                                    ctx.run,
                                    ctx.bus,
                                );
                                self.handle_shop_action_result(result, ctx.cursor_pos, ctx.bus);
                                self.focus = None;
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
                if let Some(ShopFocus::Relic(i)) = self.focus {
                    let n_for_sale = self.items.len();
                    if i >= n_for_sale {
                        let owned_idx = i - n_for_sale;
                        let action = if matches!(a, UiAction::NavigateHudNext) {
                            ShopAction::MoveRelicRight(owned_idx)
                        } else {
                            ShopAction::MoveRelicLeft(owned_idx)
                        };
                        let _result = apply_shop_action(
                            action,
                            &mut self.items,
                            &mut self.zodiac_items,
                            &mut self.talisman_items,
                            &mut self.pack_item,
                            ctx.run,
                            ctx.bus,
                        );
                        // Update focus to follow the moved relic.
                        if matches!(a, UiAction::NavigateHudNext)
                            && owned_idx + 1 < ctx.run.relics.active.len()
                        {
                            self.focus = Some(ShopFocus::Relic(n_for_sale + owned_idx + 1));
                        } else if matches!(a, UiAction::NavigateHudPrev) && owned_idx > 0 {
                            self.focus = Some(ShopFocus::Relic(n_for_sale + owned_idx - 1));
                        }
                    }
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
                        return Some(self.continue_scene(ctx.run));
                    }
                    if matches!(focus, ShopFocus::Reroll)
                        && self.mode == ShopMode::Standard
                        && ctx.run.gold >= self.reroll_cost as i32
                    {
                        self.reroll(ctx.run);
                        continue;
                    }
                    if let Some(hit) = focus.to_hit() {
                        if let Some(action) = shop_action_for_hit(
                            hit,
                            &self.items,
                            &self.zodiac_items,
                            &self.talisman_items,
                            ctx.run,
                        ) {
                            let result = apply_shop_action(
                                action,
                                &mut self.items,
                                &mut self.zodiac_items,
                                &mut self.talisman_items,
                                &mut self.pack_item,
                                ctx.run,
                                ctx.bus,
                            );
                            self.handle_shop_action_result(result, ctx.cursor_pos, ctx.bus);
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
                return Some(self.continue_scene(ctx.run));
            }
        }
        for &cid in ctx.button_clicks {
            if cid >= SHOP_SELL_RELIC_BASE && cid < SHOP_SELL_RELIC_BASE + 64 {
                let idx = (cid - SHOP_SELL_RELIC_BASE) as usize;
                let result = apply_shop_action(
                    ShopAction::SellRelic(idx),
                    &mut self.items,
                    &mut self.zodiac_items,
                    &mut self.talisman_items,
                    &mut self.pack_item,
                    ctx.run,
                    ctx.bus,
                );
                self.handle_shop_action_result(result, ctx.cursor_pos, ctx.bus);
                self.focus = None;
                return None;
            }
            if cid >= SHOP_SELL_CONSUMABLE_BASE && cid < SHOP_SELL_CONSUMABLE_BASE + 32 {
                let idx = (cid - SHOP_SELL_CONSUMABLE_BASE) as usize;
                let result = apply_shop_action(
                    ShopAction::SellConsumable(idx),
                    &mut self.items,
                    &mut self.zodiac_items,
                    &mut self.talisman_items,
                    &mut self.pack_item,
                    ctx.run,
                    ctx.bus,
                );
                self.handle_shop_action_result(result, ctx.cursor_pos, ctx.bus);
                self.focus = None;
                return None;
            }
            if cid == SHOP_NEXT_ROUND_ID {
                return Some(self.continue_scene(ctx.run));
            }
            if cid == SHOP_REROLL_ID
                && self.mode == ShopMode::Standard
                && ctx.run.gold >= self.reroll_cost as i32
            {
                self.reroll(ctx.run);
                return None;
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
            if let Some(action) = shop_action_for_hit(
                hit,
                &self.items,
                &self.zodiac_items,
                &self.talisman_items,
                ctx.run,
            ) {
                let result = apply_shop_action(
                    action,
                    &mut self.items,
                    &mut self.zodiac_items,
                    &mut self.talisman_items,
                    &mut self.pack_item,
                    ctx.run,
                    ctx.bus,
                );
                self.handle_shop_action_result(result, ctx.cursor_pos, ctx.bus);
            }
            return None;
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;
        let n_for_sale_zodiacs = self.zodiac_items.len();
        let n_for_sale_talismans = self.talisman_items.len();
        let n_owned_relics = ctx.run.relics.active.len();
        let n_owned_consumables = ctx.run.consumables.items.len();
        let layout = ShopLayout::build(
            ctx.layout,
            self.items.len(),
            n_for_sale_zodiacs,
            n_for_sale_talismans,
            n_owned_relics,
            n_owned_consumables,
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
        frame.ember_drift();
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
        // Consumable tray — owned zodiac ribbons + talisman tablets.
        frame.dish_explicit(DishExplicit {
            center_pos: [
                layout.consumable_row_center_px.0,
                layout.consumable_row_center_px.1,
                layout.consumable_row_center_px.2,
            ],
            extents: layout.consumable_row_extents,
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
        // shelf as a standing book (rounded spine, page inset).
        // Clicking it opens `JournalOverlay`.
        let journal_cx = w * 0.13;
        let journal_cy = h * 0.62;
        let journal_cz = h * 0.05;
        let journal_extents = [w * 0.018, h * 0.06, w * 0.025];
        frame.book(BookPlacement {
            world_pos: [journal_cx, journal_cy, journal_cz],
            rotation_y: 0.15, // slight yaw so the spine catches the light
            half_extents: [journal_extents[0], journal_extents[1], journal_extents[2]],
            color: [0.30, 0.12, 0.08, 1.0], // oxblood leather
            pick_id: Some(PICK_JOURNAL_BOOK),
        });

        // Tile pack — displayed in the cabinet, leaning against the back
        // of the shelf with its textured front face visible.
        if let Some(ref pack) = self.pack_item {
            if !pack.sold {
                let ext = layout.pack_extents;
                frame.pack_batch(vec![PackPlacement {
                    world_pos: [
                        layout.pack_center_px.0,
                        layout.pack_center_px.1,
                        layout.pack_center_px.2,
                    ],
                    half_extents: [ext[0] * 0.5, ext[1] * 0.5, ext[2] * 0.5],
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: pack.kind,
                    rotation_x_deg: -5.0,
                    rotation_y_deg: 0.0,
                    pick_id: Some(PICK_TILE_PACK),
                }]);
            }
        }

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
                rotation_x_deg: 0.0,
                rotation_z_deg: 0.0,
            });
        }
        let owned_base = layout.relic_dish_extents[0] * 0.15;
        for (i, &rid) in ctx.run.relics.active.iter().enumerate() {
            let (px, py, wy) = layout.owned_relic_pos(i);
            let rarity = all_relic_defs()
                .iter()
                .find(|d| d.id == rid)
                .map(|d| d.rarity)
                .unwrap_or(Rarity::Common);
            let half = relic_half_extents(rid, owned_base);
            let (glow, wiggle_deg) = if let Some(start) = self.relic_glow_starts.get(&rid) {
                let age = Instant::now()
                    .saturating_duration_since(*start)
                    .as_secs_f32();
                let life = RELIC_GLOW_LIFETIME.as_secs_f32();
                if age >= life {
                    (0.0, 0.0)
                } else {
                    let t = (age / life).clamp(0.0, 1.0);
                    let attack_end = 0.12_f32;
                    let g = if t < attack_end {
                        (t / attack_end).clamp(0.0, 1.0)
                    } else {
                        let decay_t = (t - attack_end) / (1.0 - attack_end);
                        (1.0 - decay_t).max(0.0).powi(2)
                    };
                    (g, g * 12.0 * (age * 22.0).sin())
                }
            } else {
                (0.0, 0.0)
            };
            relic_placements.push(RelicPlacement {
                world_pos: [px, py, wy],
                half_extents: half,
                color: rarity_color(rarity),
                relic_id: rid,
                glow,
                rotation_x_deg: 0.0,
                rotation_z_deg: wiggle_deg,
            });
        }
        if !relic_placements.is_empty() {
            frame.relic_batch(relic_placements);
        }

        // ── Consumable batches: zodiacs are silken ribbons (upper-right
        //    cabinet zone), talismans are jade octagonal tablets (lower-
        //    right cabinet zone, below the shelf divider). Each gets its
        //    own batch, pick path, and dedicated wall/tray positions.
        let mut ribbon_placements: Vec<ZodiacRibbonPlacement> = Vec::new();
        let mut talisman_placements: Vec<TalismanPlacement> = Vec::new();

        // For-sale zodiacs: upper-right cabinet wall.
        for (i, item) in self.zodiac_items.iter().enumerate() {
            if i >= layout.ribbon_count {
                break;
            }
            let (ax, ay, awy) = layout.ribbon_anchors_px[i];
            let mut col = consumable_color(item.consumable);
            if item.sold {
                col[3] = 0.30;
            }
            let alpha = col[3];
            ribbon_placements.push(ZodiacRibbonPlacement {
                anchor_pos: [ax, ay, awy],
                length: layout.ribbon_length,
                width: layout.ribbon_width,
                rotation_y_deg: 0.0,
                rotation_x_deg: 0.0,
                rotation_z_deg: 0.0,
                color: [1.0, 1.0, 1.0, alpha],
                kind: if let Consumable::Zodiac(z) = item.consumable {
                    Some(z)
                } else {
                    None
                },
            });
        }

        // For-sale talismans: lower-right cabinet wall (below shelf divider).
        for (i, item) in self.talisman_items.iter().enumerate() {
            if i >= layout.talisman_anchor_count {
                break;
            }
            let (ax, ay, awy) = layout.talisman_anchors_px[i];
            let mut col = consumable_color(item.consumable);
            if item.sold {
                col[3] = 0.30;
            }
            if let Consumable::Talisman(tk) = item.consumable {
                talisman_placements.push(TalismanPlacement {
                    center_pos: [ax, ay, awy],
                    extents: [
                        layout.talisman_wall_width * 1.4,
                        layout.talisman_wall_width * 2.0,
                        layout.talisman_wall_width * 0.35,
                    ],
                    rotation_y_deg: 0.0,
                    rotation_x_deg: 0.0,
                    rotation_z_deg: 0.0,
                    color: col,
                    kind: tk,
                });
            }
        }

        // Owned consumables: flat row in the shared consumable tray.
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
                        rotation_x_deg: -90.0,
                        rotation_z_deg: 0.0,
                        color: [1.0, 1.0, 1.0, 1.0],
                        kind: Some(*z),
                    });
                }
                Consumable::Talisman(tk) => {
                    talisman_placements.push(TalismanPlacement {
                        center_pos: [ax, ay, awy - layout.consumable_length * 0.4],
                        extents: [
                            layout.consumable_width * 1.4,
                            layout.consumable_width * 2.0,
                            layout.consumable_width * 0.35,
                        ],
                        rotation_y_deg: 0.0,
                        rotation_x_deg: -90.0,
                        rotation_z_deg: 0.0,
                        color: consumable_color(*c),
                        kind: *tk,
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

        // ── Gold display: bars + coin strings inside the coin dish ─────
        let (bars, coins) = coin_display_layout(
            ctx.run.gold.max(0) as u32,
            layout.coin_dish_center_px,
            layout.coin_dish_extents,
            self.age_secs,
        );
        if !bars.is_empty() {
            frame.gold_bar_batch(bars);
        }
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
            // Coin dish sparkle light — raised above the tallest coin
            // strings/bars, tighter radius to concentrate on the gold,
            // with a breathing intensity pulse for an always-on sparkle.
            PointLight {
                pos: [
                    layout.coin_dish_center_px.0,
                    layout.coin_dish_center_px.1,
                    h * 0.24,
                ],
                radius: h * 0.38,
                color: [1.00, 0.94, 0.55],
                intensity: 3.40 + 0.20 * (self.age_secs * 0.7).sin(),
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
        let hover = self
            .focus
            .and_then(|f| f.to_hit())
            .or(ctx.picked_shop_object)
            .and_then(|hit| {
                live_shop_hit(
                    hit,
                    &self.items,
                    &self.zodiac_items,
                    &self.talisman_items,
                    &self.pack_item,
                    ctx.run,
                )
            });
        if let Some(hit) = hover {
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            // Helper: get the (px, py, wy) anchor of a hit consumable for
            // spotlight placement. Walks the same partition the renderer
            // uses (for-sale-of-kind, then owned-of-kind) to find which
            // wall slot or fan position to light up.
            let zodiac_anchor = |hit_idx: usize| -> Option<(f32, f32, f32)> {
                let n_for_sale = self.zodiac_items.len();
                if hit_idx < n_for_sale {
                    if hit_idx < layout.ribbon_count {
                        return Some(layout.ribbon_anchors_px[hit_idx]);
                    }
                    return None;
                }
                // Owned zodiac: find its position in the shared consumable tray.
                let owned_target = hit_idx - n_for_sale;
                let mut count = 0usize;
                for (row_i, c) in ctx.run.consumables.items.iter().enumerate() {
                    if matches!(c, Consumable::Zodiac(_)) {
                        if count == owned_target {
                            if row_i < layout.consumable_count {
                                return Some(layout.consumable_pos(row_i));
                            }
                            return None;
                        }
                        count += 1;
                    }
                }
                None
            };
            let talisman_anchor = |hit_idx: usize| -> Option<(f32, f32, f32)> {
                let n_for_sale = self.talisman_items.len();
                if hit_idx < n_for_sale {
                    if hit_idx < layout.talisman_anchor_count {
                        return Some(layout.talisman_anchors_px[hit_idx]);
                    }
                    return None;
                }
                // Owned talisman: find its position in the shared consumable tray.
                let owned_target = hit_idx - n_for_sale;
                let mut count = 0usize;
                for (row_i, c) in ctx.run.consumables.items.iter().enumerate() {
                    if matches!(c, Consumable::Talisman(_)) {
                        if count == owned_target {
                            if row_i < layout.consumable_count {
                                return Some(layout.consumable_pos(row_i));
                            }
                            return None;
                        }
                        count += 1;
                    }
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
                    if let Some((px, py, wy)) = zodiac_anchor(i) {
                        point_lights.push(PointLight {
                            pos: [px, py + 40.0, wy - layout.ribbon_length * 0.4],
                            radius: 200.0,
                            color: [1.00, 0.92, 0.74],
                            intensity: 3.00,
                        });
                    }
                }
                ShopHit::Talisman(i) => {
                    if let Some((px, py, wy)) = talisman_anchor(i) {
                        point_lights.push(PointLight {
                            pos: [px, py + 30.0, wy + 40.0],
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
                ShopHit::TilePack(_) => {
                    point_lights.push(PointLight {
                        pos: [
                            layout.pack_center_px.0,
                            layout.pack_center_px.1 - 30.0,
                            layout.pack_center_px.2 + 60.0,
                        ],
                        radius: 180.0,
                        color: [1.00, 0.92, 0.70],
                        intensity: 3.20,
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
        let plaque_screen = ctx.proj.plaque_rects.first().copied().unwrap_or_else(|| {
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
        let n_for_sale_relics = self.items.len().min(layout.niche_count);
        // Suppress unused-binding warning when score_panel was the previous
        // anchor and is no longer used here.
        let _ = ctx.layout.score_panel;

        // Tooltip on the hovered object.
        if let Some(hit) = hover {
            let n_for_sale_zodiacs = self.zodiac_items.len();
            let n_for_sale_talismans = self.talisman_items.len();
            // Look up the projected screen rect from the renderer.
            let tooltip_anchor: Option<[f32; 4]> = match hit {
                ShopHit::Relic(i) => ctx.proj.relic_rects.get(i).copied(),
                ShopHit::Ribbon(i) => ctx.proj.ribbon_rects.get(i).copied(),
                ShopHit::Talisman(i) => ctx.proj.talisman_rects.get(i).copied(),
                ShopHit::Dish(id) | ShopHit::TilePack(id) => ctx
                    .proj
                    .aux_dish_rects
                    .iter()
                    .find_map(|(pid, r)| if *pid == Some(id) { Some(*r) } else { None }),
            };
            // Build the tooltip body lines based on hit kind.
            let (title, subtitle, cta, cta_color) = match hit {
                ShopHit::Relic(i) if i < n_for_sale_relics => {
                    let item = &self.items[i];
                    let can_afford = ctx.run.gold >= item.price as i32
                        && !ctx.run.relics.is_full()
                        && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        if ctx.run.relics.is_full() {
                            "Relics full".to_string()
                        } else {
                            format!("Need {}g", item.price as i32 - ctx.run.gold)
                        }
                    } else {
                        item.buy_label()
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
                        let name = def
                            .map(|d| d.name.to_string())
                            .unwrap_or_else(|| "Relic".into());
                        let desc = relic_description_live(
                            rid,
                            &ctx.run.relic_counters,
                            ctx.run.total_score_earned,
                        );
                        let sell = relic_sell_price_live(rid, &ctx.run.relic_counters);
                        (name, desc, format!("Sell {}g", sell), color::CHAMPAGNE)
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Ribbon(i) if i < n_for_sale_zodiacs => {
                    let item = &self.zodiac_items[i];
                    let price = item.price();
                    let can_afford = ctx.run.gold >= price as i32 && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        format!("Need {}g", price as i32 - ctx.run.gold)
                    } else if price == 0 {
                        "FREE".to_string()
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
                            "Use".to_string(),
                            color::CHAMPAGNE,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Talisman(i) if i < n_for_sale_talismans => {
                    let item = &self.talisman_items[i];
                    let price = item.price();
                    let can_afford = ctx.run.gold >= price as i32
                        && !ctx.run.consumables.is_full()
                        && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        if ctx.run.consumables.is_full() {
                            "Inventory full".to_string()
                        } else {
                            format!("Need {}g", price as i32 - ctx.run.gold)
                        }
                    } else if price == 0 {
                        "FREE".to_string()
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
                ShopHit::Dish(id) if id == PICK_TILE_PACK => {
                    if let Some(ref pack) = self.pack_item {
                        let price = pack.kind.shop_price();
                        let can_afford = ctx.run.gold >= price as i32 && !pack.sold;
                        let cta = if pack.sold {
                            "SOLD".to_string()
                        } else if price == 0 {
                            "FREE".to_string()
                        } else if can_afford {
                            format!("Buy {}g", price)
                        } else {
                            format!("Need {}g", price as i32 - ctx.run.gold)
                        };
                        (
                            pack.kind.name().to_string(),
                            pack.kind.description().to_string(),
                            cta,
                            color::CHAMPAGNE,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Dish(_) => (
                    "Relic dish".to_string(),
                    "Hover an owned relic to sell it".to_string(),
                    String::new(),
                    color::SLATE,
                ),
                ShopHit::TilePack(_id) => {
                    if let Some(ref pack) = self.pack_item {
                        let price = pack.kind.shop_price();
                        let can_afford = ctx.run.gold >= price as i32 && !pack.sold;
                        let cta = if pack.sold {
                            "SOLD".to_string()
                        } else if price == 0 {
                            "FREE".to_string()
                        } else if can_afford {
                            format!("Buy {}g", price)
                        } else {
                            format!("{}g (not enough gold)", price)
                        };
                        let col = if pack.sold {
                            color::SLATE
                        } else if can_afford {
                            color::CHAMPAGNE
                        } else {
                            color::SLATE
                        };
                        (
                            pack.kind.name().to_string(),
                            pack.kind.description().to_string(),
                            cta,
                            col,
                        )
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
            };
            if !title.is_empty() {
                if let Some(rect) = tooltip_anchor {
                    let pad = 16.0_f32;
                    // Pin font sizes explicitly (don't let the rasterizer
                    // auto-shrink) so they match what the typography tiers
                    // are supposed to produce at this window height.
                    let title_font = typography::size(typography::TITLE, h, ui_scale).max(22.0);
                    let body_font = typography::size(typography::BODY, h, ui_scale).max(16.0);
                    let cta_font = typography::size(typography::HEADING, h, ui_scale).max(20.0);
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
                        ui_scale,
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

        if let Some(hit) = hover {
            match hit {
                ShopHit::Relic(i) if i >= n_for_sale_relics => {
                    let owned_idx = i - n_for_sale_relics;
                    if let Some(rect) = ctx.proj.relic_rects.get(i).copied() {
                        let rid = ctx.run.relics.active[owned_idx];
                        let refund = relic_sell_price_live(rid, &ctx.run.relic_counters);
                        let sell_rect = [
                            rect[0] + rect[2] * 0.46,
                            rect[1] + 6.0,
                            rect[2] * 0.48,
                            24.0,
                        ];
                        widget::push_button(
                            &mut quads,
                            &mut texts,
                            &mut buttons,
                            sell_rect,
                            &format!("Sell {refund}g"),
                            ButtonVariant::Default,
                            ButtonState::Rest,
                            UiAction::Cancel,
                        );
                        buttons.pop();
                        buttons.push(ButtonDef::scene(
                            (sell_rect[0], sell_rect[1], sell_rect[2], sell_rect[3]),
                            SHOP_SELL_RELIC_BASE + owned_idx as u32,
                        ));
                    }
                }
                ShopHit::Ribbon(i) => {
                    if let Some(inv_idx) =
                        owned_ribbon_inventory_index(i, &self.zodiac_items, ctx.run)
                    {
                        if let Some(rect) = ctx.proj.ribbon_rects.get(i).copied() {
                            let refund = consumable_sell_price(ctx.run.consumables.items[inv_idx]);
                            let sell_rect = [
                                rect[0] + rect[2] * 0.40,
                                rect[1] + 6.0,
                                rect[2] * 0.56,
                                24.0,
                            ];
                            widget::push_button(
                                &mut quads,
                                &mut texts,
                                &mut buttons,
                                sell_rect,
                                &format!("Sell {refund}g"),
                                ButtonVariant::Default,
                                ButtonState::Rest,
                                UiAction::Cancel,
                            );
                            buttons.pop();
                            buttons.push(ButtonDef::scene(
                                (sell_rect[0], sell_rect[1], sell_rect[2], sell_rect[3]),
                                SHOP_SELL_CONSUMABLE_BASE + inv_idx as u32,
                            ));
                        }
                    }
                }
                ShopHit::Talisman(i) => {
                    if let Some(inv_idx) =
                        owned_talisman_inventory_index(i, &self.talisman_items, ctx.run)
                    {
                        if let Some(rect) = ctx.proj.talisman_rects.get(i).copied() {
                            let refund = consumable_sell_price(ctx.run.consumables.items[inv_idx]);
                            let sell_rect = [
                                rect[0] + rect[2] * 0.36,
                                rect[1] + 6.0,
                                rect[2] * 0.60,
                                24.0,
                            ];
                            widget::push_button(
                                &mut quads,
                                &mut texts,
                                &mut buttons,
                                sell_rect,
                                &format!("Sell {refund}g"),
                                ButtonVariant::Default,
                                ButtonState::Rest,
                                UiAction::Cancel,
                            );
                            buttons.pop();
                            buttons.push(ButtonDef::scene(
                                (sell_rect[0], sell_rect[1], sell_rect[2], sell_rect[3]),
                                SHOP_SELL_CONSUMABLE_BASE + inv_idx as u32,
                            ));
                        }
                    }
                }
                _ => {}
            }
        }

        // ── Tutorial shop banner ────────────────────────────────────────
        // When the tutorial is active and the lesson is shop-enabled, show
        // a hint banner at the top of the screen guiding the player through
        // the shop UI — mirroring the gameplay scene's tutorial overlay
        // style.
        if self.mode == ShopMode::Tutorial {
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            let n_for_sale_zodiacs = self.zodiac_items.len();
            let n_for_sale_talismans = self.talisman_items.len();
            let has_bought = !ctx.run.relics.active.is_empty()
                || !ctx.run.consumables.items.is_empty()
                || ctx
                    .run
                    .yaku_levels
                    .level_of(crate::core::yaku::YakuKind::FullHand)
                    > 1
                || self.pack_item.as_ref().is_some_and(|p| p.sold);
            let (flavor, hint) = if has_bought {
                (
                    "Your loadout is ready.",
                    "Try selling an owned item if you want to see the refund flow, or use LB / RB on owned relics to reorder them before you face The Iconoclast.",
                )
            } else if self.items.is_empty() {
                (
                    "The cabinet is bare\u{2026}",
                    "Press Next Round to move on.",
                )
            } else if let Some(hit) = hover {
                match hit {
                    ShopHit::Relic(i) if i < n_for_sale_relics => (
                        "Relics are permanent run upgrades.",
                        "This cabinet sells passive relics. Read the tooltip, check the gold cost, and buy the one that best helps your scoring plan.",
                    ),
                    ShopHit::Relic(_) => (
                        "Owned relics live in the lower dish.",
                        "Hover a relic in the dish to review its effect. Use the Sell button or press LB / [ to cash it out if you want to pivot your build.",
                    ),
                    ShopHit::Ribbon(i) if i < n_for_sale_zodiacs => (
                        "Ribbons level up a yaku.",
                        "Buying a ribbon boosts one scoring pattern for the rest of the run. They are great when you already know which yaku you want to chase.",
                    ),
                    ShopHit::Talisman(i) if i < n_for_sale_talismans => (
                        "Talismans are consumable upgrades.",
                        "Talismans go into your consumable tray and modify tiles or scoring. They are flexible pickups when you do not want to commit to a relic.",
                    ),
                    ShopHit::Ribbon(_) => (
                        "Owned ribbons can be used here.",
                        "Hover an owned ribbon in the tray and click Use to apply its yaku level-up before the next blind.",
                    ),
                    ShopHit::Talisman(_) => (
                        "Owned talismans can be sold back.",
                        "Hover an owned talisman in the tray to inspect it or sell it for gold if you need room.",
                    ),
                    ShopHit::Dish(id) if id == PICK_TILE_PACK => (
                        "Tile packs change the wall.",
                        "Packs add new tiles to future draws. They are optional, but can reshape the kinds of melds your run wants to make.",
                    ),
                    ShopHit::TilePack(_) => (
                        "Tile packs change the wall.",
                        "Packs add new tiles to future draws. They are optional, but can reshape the kinds of melds your run wants to make.",
                    ),
                    _ => (
                        "Take a look around the Shop.",
                        "Hover any item to inspect it. The tooltip tells you what it does and whether you can buy, use, or sell it.",
                    ),
                }
            } else {
                (
                    "Welcome to the Shop!",
                    "Left cabinet: relics for passive bonuses. Hanging ribbons: yaku level-ups. Jade tablets: talismans. Hover anything to inspect it, then buy what helps before pressing Next Round.",
                )
            };

            let alpha = 1.0_f32;
            let flavor_px = typography::size(typography::BODY, h, ui_scale).max(15.0);
            let hint_px = typography::size(typography::BODY, h, ui_scale).max(15.0);
            let pad = (16.0 * ui_scale).max(10.0);

            // Right-side vertical panel — sits below the zodiac area so
            // it never overlaps relic tooltips in the upper-left.
            let banner_w = (w * 0.30).clamp(320.0, 460.0);
            let banner_x = w - banner_w - w * 0.02;
            let banner_y = h * 0.40;
            let text_w = banner_w - pad * 2.0;

            // Pre-wrap both text blocks to compute dynamic height.
            let flavor_line_h = flavor_px * 1.4;
            let flavor_lines = widget::wrap_text(flavor, text_w, flavor_px);
            let flavor_h = flavor_lines.len().max(1) as f32 * flavor_line_h;
            let hint_line_h = hint_px * 1.4;
            let hint_lines = widget::wrap_text(hint, text_w, hint_px);
            let hint_h = hint_lines.len().max(1) as f32 * hint_line_h;
            let banner_h = (pad + flavor_h + pad * 0.5 + hint_h + pad)
                .min(h - banner_y - (92.0 * ui_scale).max(72.0));

            // Gold border.
            let border = 2.0;
            quads.push(GpuInstance {
                rect: [
                    banner_x - border,
                    banner_y - border,
                    banner_w + border * 2.0,
                    banner_h + border * 2.0,
                ],
                color: [
                    color::BRASS[0],
                    color::BRASS[1],
                    color::BRASS[2],
                    0.4 * alpha,
                ],
            });
            // Dark panel.
            quads.push(GpuInstance {
                rect: [banner_x, banner_y, banner_w, banner_h],
                color: [
                    color::MIDNIGHT[0],
                    color::MIDNIGHT[1],
                    color::MIDNIGHT[2],
                    0.88 * alpha,
                ],
            });
            // Flavor text (gold, left-aligned for narrow panel).
            let flavor_y = banner_y + pad;
            widget::push_text_block(
                &mut texts,
                [banner_x + pad, flavor_y, text_w, flavor_h],
                flavor,
                TextStyle {
                    tier: typography::BODY,
                    color: [color::GOLD[0], color::GOLD[1], color::GOLD[2], 0.8 * alpha],
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
                ui_scale,
            );
            // Hint text (champagne, left-aligned).
            let hint_y = flavor_y + flavor_h + pad * 0.5;
            widget::push_text_block(
                &mut texts,
                [banner_x + pad, hint_y, text_w, hint_h],
                hint,
                TextStyle {
                    tier: typography::BODY,
                    color: [
                        color::CHAMPAGNE[0],
                        color::CHAMPAGNE[1],
                        color::CHAMPAGNE[2],
                        alpha,
                    ],
                    padding: 0.0,
                    align: TextAlign::Left,
                },
                h,
                ui_scale,
            );
        }

        // ── Reroll + Next Round buttons (always-visible, 2D) ──────────
        let scale = metrics::scene_scale(w, h, ui_scale);
        let btn_w = (180.0 * scale).max(120.0);
        let btn_h = (44.0 * scale).max(28.0);
        let btn_gap = 12.0 * scale;
        let total_w = btn_w * 2.0 + btn_gap;
        let left_x = (w - total_w) * 0.5;
        let btn_y = h - btn_h - (16.0 * scale);

        // Reroll button (left).
        let reroll_affordable =
            self.mode == ShopMode::Standard && ctx.run.gold >= self.reroll_cost as i32;
        let reroll_state = if reroll_affordable {
            ButtonState::Rest
        } else {
            ButtonState::Disabled
        };
        let reroll_label = if self.mode == ShopMode::Tutorial {
            "Curated Stock".to_string()
        } else {
            format!("Restock ${}g", self.reroll_cost)
        };
        widget::push_button(
            &mut quads,
            &mut texts,
            &mut buttons,
            [left_x, btn_y, btn_w, btn_h],
            &reroll_label,
            ButtonVariant::Default,
            reroll_state,
            UiAction::Cancel, // placeholder — replaced below
        );
        buttons.pop();
        buttons.push(ButtonDef::scene(
            (left_x, btn_y, btn_w, btn_h),
            SHOP_REROLL_ID,
        ));
        let reroll_rect: [f32; 4] = [left_x, btn_y, btn_w, btn_h];

        // Next Round button (right).
        let btn_x = left_x + btn_w + btn_gap;
        widget::push_button(
            &mut quads,
            &mut texts,
            &mut buttons,
            [btn_x, btn_y, btn_w, btn_h],
            if self.mode == ShopMode::Tutorial {
                "Face Boss"
            } else {
                "Next Round"
            },
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
        for (i, r) in ctx.proj.relic_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((ShopFocus::Relic(i), *r));
            }
        }
        for (i, r) in ctx.proj.ribbon_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((ShopFocus::Ribbon(i), *r));
            }
        }
        for (i, r) in ctx.proj.talisman_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((ShopFocus::Talisman(i), *r));
            }
        }
        for (pid, r) in ctx.proj.aux_dish_rects.iter() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                if let Some(id) = pid {
                    if *id == PICK_TILE_PACK {
                        focus_rect_graph.push((ShopFocus::Pack(*id), *r));
                    } else {
                        focus_rect_graph.push((ShopFocus::Dish(*id), *r));
                    }
                }
            }
        }
        focus_rect_graph.push((ShopFocus::Reroll, reroll_rect));
        focus_rect_graph.push((ShopFocus::NextRound, next_round_rect));

        // Push the brass focus ring on top of the 2D HUD layer so it
        // sits above the cabinet wood and dishes. Skipped during pause /
        // overlay states because the overlay's own buttons take focus.
        if !self.pause_menu.paused && !self.journal.open {
            if let Some(target) = self.focus {
                let rect_lookup = focus_rect_graph
                    .iter()
                    .find_map(|(t, r)| (*t == target).then_some(*r));
                if let Some(rect) = rect_lookup {
                    push_focus_ring(rect, scale, &mut quads);
                }
            }
        }

        for (i, item) in self.items.iter().enumerate() {
            if item.sold || item.price != 0 {
                continue;
            }
            if let Some(rect) = ctx.proj.relic_rects.get(i).copied() {
                push_free_badge(&mut quads, &mut texts, rect, h, ui_scale);
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
            .draw(w, h, scale, &mut quads, &mut texts, &mut buttons, ui_scale);
        // Fullscreen click-blocker behind the pause menu's own buttons so
        // missed clicks become no-ops instead of falling through.
        if self.pause_menu.paused {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Yaku Journal overlay (drawn last so it covers everything).
        self.journal.draw(
            w,
            h,
            ui_scale,
            ctx.run,
            &mut quads,
            &mut texts,
            &mut buttons,
        );
        if self.journal.open {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Push 2D layers onto the frame after all 3D content.
        frame.quads(quads);
        frame.texts(texts);

        // ── Zodiac ribbon close-up celebration overlay ──────────────
        if let Some(ref celeb) = self.zodiac_celebration {
            // Semi-transparent dimmer.
            frame.quad(GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: [0.0, 0.0, 0.0, 0.72],
            });
            // Constellation starfield vignette behind the ribbon.
            frame.starfield();

            let t = celeb.elapsed();
            // Ribbon dimensions — large and centered.
            let ribbon_w = h * 0.12;
            let ribbon_l = h * 0.55;
            let cx = w * 0.5;
            let cy = h * 0.35;
            let lift = h * 0.50;

            // ── Snake / sway animation ──────────────────────────────
            // Primary sway: slow sinusoidal yaw oscillation.
            let sway_yaw = (t * 1.8).sin() * 12.0; // degrees
            // Secondary roll: faster, smaller, phase-offset from yaw
            // so the ribbon traces an organic figure-eight path.
            let sway_roll = (t * 2.5 + 0.7).sin() * 6.0; // degrees
            // Gentle forward tilt so the face catches the light.
            let tilt = 8.0 + (t * 1.2).sin() * 3.0; // degrees

            // Fade-in: quick opacity ramp over the first 0.3s.
            let alpha = (t / 0.3).clamp(0.0, 1.0);

            frame.zodiac_batch(vec![ZodiacRibbonPlacement {
                anchor_pos: [cx, cy, lift],
                length: ribbon_l,
                width: ribbon_w,
                rotation_y_deg: sway_yaw,
                rotation_x_deg: tilt,
                rotation_z_deg: sway_roll,
                color: [1.0, 1.0, 1.0, alpha],
                kind: Some(celeb.kind),
            }]);

            // Title: yaku name above the ribbon.
            let title_font = (h * 0.04).max(24.0);
            let title_y = h * 0.10;
            frame.text(TextLabel {
                text: format!("{} Lvl.{}", celeb.yaku_name, celeb.new_level),
                rect: [0.0, title_y, w, title_font * 1.5],
                font_px: Some(title_font),
                color: [0.95, 0.78, 0.25, alpha],
                align: TextAlign::Center,
                ..Default::default()
            });

            // Dismiss prompt — pulsing at the bottom.
            let prompt_font = (h * 0.028).max(18.0);
            let prompt_y = h * 0.88;
            let pulse_alpha = alpha * (0.5 + 0.5 * (t * 3.0).sin());
            frame.text(TextLabel {
                text: "Click or press confirm to continue".to_string(),
                rect: [0.0, prompt_y, w, prompt_font * 1.5],
                font_px: Some(prompt_font),
                color: [1.0, 1.0, 1.0, pulse_alpha],
                align: TextAlign::Center,
                ..Default::default()
            });

            // Block all shop buttons so clicks go to the celebration.
            buttons.clear();
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), SHOP_3D_HIT_ID));
        }

        // ── Tile-pack opening celebration overlay ─────────────────────
        if let Some(ref celeb) = self.pack_celebration {
            let n = celeb.tiles.len();
            // Semi-transparent dimmer over the whole shop.
            frame.quad(GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: [0.0, 0.0, 0.0, 0.72],
            });

            // Title: pack name — above the content area.
            let title_font = (h * 0.045).max(28.0);
            let title_y = h * 0.18;
            frame.text(TextLabel {
                text: celeb.pack_name.to_string(),
                rect: [0.0, title_y, w, title_font * 1.5],
                font_px: Some(title_font),
                color: color::CHAMPAGNE,
                align: TextAlign::Center,
                ..Default::default()
            });

            match celeb.phase {
                CelebPhase::Closeup => {
                    // ── Closeup: large pack box centered on screen ────
                    // Rendered via PackPlacement so it gets the foil
                    // material + texture. Gently bobs in place.
                    let box_h = h * 0.28;
                    let box_w = box_h; // square front face
                    let box_d = box_h * 0.25;
                    let t = celeb.started_at.elapsed().as_secs_f32();
                    // Calm, slow bob using incommensurate frequencies so
                    // the motion feels organic, not mechanical.
                    let bob_x = (t * 0.7).sin() * h * 0.008;
                    let bob_y = (t * 0.5).sin() * h * 0.006;
                    let bob_rx = (t * 0.6).sin() * 2.5; // degrees
                    let bob_ry = (t * 0.8).cos() * 3.0; // degrees
                    frame.pack_batch(vec![PackPlacement {
                        world_pos: [w * 0.5 + bob_x, h * 0.45, h * 0.28 + bob_y],
                        half_extents: [box_w * 0.5, box_h * 0.5, box_d * 0.5],
                        color: [1.0, 1.0, 1.0, 1.0],
                        kind: celeb.pack_kind,
                        rotation_x_deg: bob_rx,
                        rotation_y_deg: bob_ry,
                        pick_id: None,
                    }]);

                    // "Click to open" prompt at the bottom.
                    let prompt_font = (h * 0.028).max(18.0);
                    let prompt_y = h * 0.88;
                    let t = celeb.started_at.elapsed().as_secs_f32();
                    let pulse_alpha = 0.5 + 0.5 * (t * 3.0).sin();
                    frame.text(TextLabel {
                        text: "Click or press confirm to open".to_string(),
                        rect: [0.0, prompt_y, w, prompt_font * 1.5],
                        font_px: Some(prompt_font),
                        color: [1.0, 1.0, 1.0, pulse_alpha],
                        align: TextAlign::Center,
                        ..Default::default()
                    });
                }
                CelebPhase::Reveal => {
                    // ── Reveal: tiles flying out into a row ───────────
                    let tile_size = h * 0.13;
                    let gap = tile_size * 0.25;
                    let total_w = n as f32 * tile_size + (n.saturating_sub(1)) as f32 * gap;
                    let row_x0 = (w - total_w) * 0.5;
                    let row_py = h * 0.55;
                    let row_lift = h * 0.22;
                    let src_px = w * 0.5;
                    let src_py = h * 0.35;
                    let src_lift = row_lift + h * 0.15;

                    let mut placements = Vec::with_capacity(n);
                    for i in 0..n {
                        let t = celeb.tile_progress(i);
                        let ease = 1.0 - (1.0 - t).powi(3);

                        let dest_px = row_x0 + i as f32 * (tile_size + gap) + tile_size * 0.5;
                        let px = src_px + (dest_px - src_px) * ease;
                        let py = src_py + (row_py - src_py) * ease;
                        let lift = src_lift + (row_lift - src_lift) * ease;
                        let scale = 0.3 + 0.7 * ease;

                        placements.push(ShowcaseTilePlacement {
                            tile: celeb.tiles[i].clone(),
                            center_pos: [px, py, lift],
                            rotation: [60.0_f32.to_radians(), 0.0, 0.0],
                            scale,
                            size_px: tile_size,
                            brightness: 1.0,
                            selected: false,
                        });
                    }

                    frame
                        .cmds
                        .push(crate::render::draw_cmd::DrawCmd::ShowcaseTileBatch(
                            placements,
                        ));

                    // Dismiss prompt — pinned near the bottom.
                    if celeb.fully_settled() {
                        let prompt_font = (h * 0.028).max(18.0);
                        let prompt_y = h * 0.88;
                        let pulse_alpha = 0.5 + 0.5 * ((celeb.elapsed() * 3.0).sin());
                        frame.text(TextLabel {
                            text: "Click or press confirm to continue".to_string(),
                            rect: [0.0, prompt_y, w, prompt_font * 1.5],
                            font_px: Some(prompt_font),
                            color: [1.0, 1.0, 1.0, pulse_alpha],
                            align: TextAlign::Center,
                            ..Default::default()
                        });
                    }
                }
            }

            // Block all shop buttons so clicks go to the celebration.
            buttons.clear();
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), SHOP_3D_HIT_ID));
        }

        // Zodiac level-up feedback: floating text + particles.
        let now = Instant::now();
        let glyph_placements = self.score_popups.placements(now);
        if !glyph_placements.is_empty() {
            frame.extruded_glyph_batch(glyph_placements);
        }
        for (rect, color) in self.particles.instances() {
            frame.quad(GpuInstance { rect, color });
        }

        frame.buttons = buttons;
        frame.window_title = format!(
            "Mahjuro — Shop (Round {}) — Gold: {}",
            self.came_from_round, ctx.run.gold
        );

        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::game_mode::GameMode;

    #[test]
    fn patron_gift_shop_always_contains_a_free_relic() {
        let mut run = crate::game::run::RunState::new(GameMode::standard());
        run.tag_patron_gift = true;

        let shop = ShopScene::new(1, &mut run);

        assert!(!shop.items.is_empty());
        assert!(shop.items.iter().any(|item| item.price == 0));
        assert!(!run.tag_patron_gift);
    }

    #[test]
    fn rich_stock_shop_starts_with_two_extra_relics() {
        let mut run = crate::game::run::RunState::new(GameMode::standard());
        run.tag_rich_stock = true;

        let shop = ShopScene::new(1, &mut run);

        assert!(shop.items.len() >= 2);
        assert!(!run.tag_rich_stock);
    }
}
