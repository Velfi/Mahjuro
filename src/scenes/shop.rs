//! Shop scene — between rounds; player can buy relics with gold.
//!
//! Renders as a night **mountain path kiosk**: a wide counter with four
//! back-row stalls (relics, tile pack, talismans, ribbons), owned inventory
//! in three bottom trays, and top chrome (path sign, lamp, leave). Hovering
//! an object turns on a point-light spotlight; tooltips and a central
//! selected-item panel show name + Buy/Sell CTA.

use rand::RngExt;
use rand::seq::SliceRandom;

use std::time::Instant;

use crate::core::consumable::Consumable;
use crate::core::relic::{
    Rarity, RelicId, RelicState, all_relic_defs, relic_description_live, relic_sell_price_live,
    relic_shop_price,
};
use crate::core::talisman::TalismanKind;
use crate::core::tile::Tile;
use crate::core::tile_pack::TilePackKind;
use crate::core::zodiac::ZodiacKind;
use crate::game::engine::{
    GameEngine, ShopCommand, ShopCommandData, ShopReadModel, consumable_sell_price,
};
use crate::render::decal::{load_ui_font, measure_plaque_wrap};
use crate::render::draw_cmd::{
    CameraParams, Object3d, Object3dKind, ShowcaseTilePlacement, UiFrame, camera_facing_rotation,
};
use crate::render::lamp_mesh::{BULB_Z as LAMP_BULB_LOCAL_Z, SHADE_RIM_R, shade_exclusion_radius};
use crate::render::particles::ParticleSystem;
use crate::render::score_popups::ScorePopupSystem;
use crate::render::table_transform::{rot_rx_rz_deg, rot_ry_rx_deg, rot_rz_ry_rx_deg, rot_z_rad};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, ShopHit, TextAlign, TextLabel};
use crate::ui::focus_nav::{FocusDir, focus_target_at_cursor, pick_neighbor, push_focus_ring};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::widget::{self, TextStyle};

use super::pause_menu::PauseMenu;
use super::pick_blind::PickBlindScene;
use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

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
    /// Buy the tile pack at the given index in `pack_items`.
    BuyPack(usize),
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
    /// The 2D "Leave" button (top-right) — also maps to PICK_LEAVE_PROP.
    NextRound,
    /// The 2D "Reroll" button at the bottom of the screen — also maps to PICK_REROLL_PROP.
    Reroll,
    /// The 3D sell tray prop in the inventory row.
    SellTray,
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
            ShopHit::Dish(id) if id == PICK_LEAVE_PROP => Self::NextRound,
            ShopHit::Dish(id) if id == PICK_REROLL_PROP => Self::Reroll,
            ShopHit::Dish(id) if id == PICK_SELL_TRAY => Self::SellTray,
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
            Self::NextRound => Some(ShopHit::Dish(PICK_LEAVE_PROP)),
            Self::Reroll => Some(ShopHit::Dish(PICK_REROLL_PROP)),
            Self::SellTray => Some(ShopHit::Dish(PICK_SELL_TRAY)),
        }
    }
}

/// Map a `ShopHit` (from a click *or* a focus Confirm) to the
/// corresponding `ShopAction`. Pure lookup against the current shop
/// stock + run state — no mutation. Dishes are info-only and return
/// `None`. Returns `None` for the journal book too; the caller handles
/// the journal separately because it pushes the `YakuJournalScene`
/// overlay rather than running a scene action.
fn shop_action_for_hit(
    hit: ShopHit,
    items: &[ShopItem],
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    shop: &ShopReadModel,
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
                owned_ribbon_inventory_index(i, zodiac_items, shop).map(ShopAction::UseConsumable)
            }
        }
        ShopHit::Talisman(i) => {
            if i < talisman_items.len() {
                Some(ShopAction::BuyTalisman(i))
            } else {
                None
            }
        }
        ShopHit::Dish(id) => tile_pack_index_from_pick(id).map(ShopAction::BuyPack),
        ShopHit::TilePack(id) => tile_pack_index_from_pick(id).map(ShopAction::BuyPack),
    }
}

/// Map the currently focused `ShopFocus` to the sell action for that item,
/// if the focused item is an owned relic or consumable. Returns `None` if
/// there is nothing focused or the focused item cannot be sold.
fn focused_sell_action(
    focus: Option<ShopFocus>,
    n_for_sale_relics: usize,
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    shop: &ShopReadModel,
) -> Option<ShopAction> {
    match focus? {
        ShopFocus::Relic(i) if i >= n_for_sale_relics => {
            let owned_idx = i - n_for_sale_relics;
            if owned_idx < shop.owned_relics.len() {
                Some(ShopAction::SellRelic(owned_idx))
            } else {
                None
            }
        }
        ShopFocus::Ribbon(i) if i >= zodiac_items.len() => {
            owned_ribbon_inventory_index(i, zodiac_items, shop).map(ShopAction::SellConsumable)
        }
        ShopFocus::Talisman(i) if i >= talisman_items.len() => {
            owned_talisman_inventory_index(i, talisman_items, shop).map(ShopAction::SellConsumable)
        }
        _ => None,
    }
}

/// Row of owned items that a sell action affected, used to normalize the
/// post-sell focus in [`focus_after_sell`].
#[derive(Clone, Copy)]
enum SoldRow {
    Relic,
    Ribbon,
    Talisman,
}

/// Pick the focus to adopt after an owned item is sold, so the player's
/// focus stays roughly where it was instead of snapping to `None`.
///
/// The sold item was at owned-index `sold_owned_idx` in `row`. If the
/// current focus is in the same row, it's rebased to the left neighbor
/// (or the item that shifted into the sold slot). If the row is now
/// empty, focus falls back to the sell tray. Focus in other rows is
/// left untouched so selling a ribbon doesn't yank the player off a
/// relic they were looking at.
/// Per-row counts needed to rebase focus after a sale: how many items
/// of each kind are for sale this round, and how many remain owned
/// after the sale completed.
pub struct SellRowCounts {
    pub n_for_sale_relics: usize,
    pub n_for_sale_ribbons: usize,
    pub n_for_sale_talismans: usize,
    pub owned_relics_after: usize,
    pub owned_ribbons_after: usize,
    pub owned_talismans_after: usize,
}

fn focus_after_sell(
    focus: Option<ShopFocus>,
    row: SoldRow,
    sold_owned_idx: usize,
    counts: SellRowCounts,
) -> Option<ShopFocus> {
    let SellRowCounts {
        n_for_sale_relics,
        n_for_sale_ribbons,
        n_for_sale_talismans,
        owned_relics_after,
        owned_ribbons_after,
        owned_talismans_after,
    } = counts;
    let rebase = |flat: usize, for_sale: usize, owned_after: usize| -> Option<usize> {
        if flat < for_sale {
            return Some(flat);
        }
        if owned_after == 0 {
            return None;
        }
        let cur_owned = flat - for_sale;
        let new_owned = if cur_owned > sold_owned_idx {
            cur_owned - 1
        } else if cur_owned == sold_owned_idx {
            sold_owned_idx.saturating_sub(1)
        } else {
            cur_owned
        };
        let new_owned = new_owned.min(owned_after - 1);
        Some(for_sale + new_owned)
    };
    let f = focus?;
    match (row, f) {
        (SoldRow::Relic, ShopFocus::Relic(i)) => Some(
            rebase(i, n_for_sale_relics, owned_relics_after)
                .map(ShopFocus::Relic)
                .unwrap_or(ShopFocus::SellTray),
        ),
        (SoldRow::Ribbon, ShopFocus::Ribbon(i)) => Some(
            rebase(i, n_for_sale_ribbons, owned_ribbons_after)
                .map(ShopFocus::Ribbon)
                .unwrap_or(ShopFocus::SellTray),
        ),
        (SoldRow::Talisman, ShopFocus::Talisman(i)) => Some(
            rebase(i, n_for_sale_talismans, owned_talismans_after)
                .map(ShopFocus::Talisman)
                .unwrap_or(ShopFocus::SellTray),
        ),
        _ => Some(f),
    }
}

/// Classify a sell action by its owned row and the owned-row-relative
/// index of the item being sold, peeking `run` *before* the sell applies.
/// Returns `None` for actions that aren't sells or whose target index is
/// out of range.
fn classify_sell(action: ShopAction, shop: &ShopReadModel) -> Option<(SoldRow, usize)> {
    match action {
        ShopAction::SellRelic(i) if i < shop.owned_relics.len() => Some((SoldRow::Relic, i)),
        ShopAction::SellConsumable(inv_idx) => {
            if let Some(owned_idx) = shop
                .owned_zodiacs
                .iter()
                .position(|owned| owned.inventory_index == inv_idx)
            {
                Some((SoldRow::Ribbon, owned_idx))
            } else {
                shop.owned_talismans
                    .iter()
                    .position(|owned| owned.inventory_index == inv_idx)
                    .map(|owned_idx| (SoldRow::Talisman, owned_idx))
            }
        }
        _ => None,
    }
}

/// Convert a [`ShopHit`] to the corresponding [`ShopDragSource`] if the hit
/// refers to an owned item. Returns `None` for for-sale items and dishes.
fn drag_source_from_hit(
    hit: ShopHit,
    n_for_sale_relics: usize,
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    shop: &ShopReadModel,
) -> Option<ShopDragSource> {
    match hit {
        ShopHit::Relic(i) if i >= n_for_sale_relics => {
            let owned_idx = i - n_for_sale_relics;
            (owned_idx < shop.owned_relics.len()).then_some(ShopDragSource::OwnedRelic(owned_idx))
        }
        ShopHit::Ribbon(i) if i >= zodiac_items.len() => {
            owned_ribbon_inventory_index(i, zodiac_items, shop).map(ShopDragSource::OwnedConsumable)
        }
        ShopHit::Talisman(i) if i >= talisman_items.len() => {
            owned_talisman_inventory_index(i, talisman_items, shop)
                .map(ShopDragSource::OwnedConsumable)
        }
        _ => None,
    }
}

/// Convert a [`ShopFocus`] to the corresponding [`ShopDragSource`] if the
/// focus refers to an owned item. Returns `None` otherwise.
fn drag_source_from_focus(
    focus: Option<ShopFocus>,
    n_for_sale_relics: usize,
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    shop: &ShopReadModel,
) -> Option<ShopDragSource> {
    match focus? {
        ShopFocus::Relic(i) if i >= n_for_sale_relics => {
            let owned_idx = i - n_for_sale_relics;
            (owned_idx < shop.owned_relics.len()).then_some(ShopDragSource::OwnedRelic(owned_idx))
        }
        ShopFocus::Ribbon(i) if i >= zodiac_items.len() => {
            owned_ribbon_inventory_index(i, zodiac_items, shop).map(ShopDragSource::OwnedConsumable)
        }
        ShopFocus::Talisman(i) if i >= talisman_items.len() => {
            owned_talisman_inventory_index(i, talisman_items, shop)
                .map(ShopDragSource::OwnedConsumable)
        }
        _ => None,
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
    pack_items: &mut [TilePackShopItem],
    run: &mut crate::game::run::RunState,
    bus: &mut crate::game::event_bus::EventBus,
) -> ShopActionResult {
    let mut engine = GameEngine::new(run, bus);
    match action {
        ShopAction::BuyCard(idx) => {
            if idx < items.len() {
                let item = &items[idx];
                if !item.sold {
                    let outcome = engine.dispatch_shop(ShopCommand::BuyRelic {
                        relic: item.relic,
                        price: item.price,
                    });
                    if outcome.rejection.is_none() {
                        items.remove(idx);
                    }
                }
            }
        }
        ShopAction::SellRelic(idx) => {
            let _ = engine.dispatch_shop(ShopCommand::SellRelic { index: idx });
        }
        ShopAction::MoveRelicLeft(idx) => {
            let _ = engine.dispatch_shop(ShopCommand::MoveRelicLeft { index: idx });
        }
        ShopAction::MoveRelicRight(idx) => {
            let _ = engine.dispatch_shop(ShopCommand::MoveRelicRight { index: idx });
        }
        ShopAction::BuyZodiac(idx) => {
            if idx < zodiac_items.len() {
                let item = &zodiac_items[idx];
                if !item.sold {
                    let price = item.price();
                    if let Consumable::Zodiac(z) = item.consumable {
                        let outcome =
                            engine.dispatch_shop(ShopCommand::BuyZodiac { zodiac: z, price });
                        if outcome.rejection.is_none() {
                            zodiac_items.remove(idx);
                        }
                        if let ShopCommandData::ZodiacApplied {
                            zodiac_kind,
                            yaku_name,
                            new_level,
                        } = outcome.data
                        {
                            return ShopActionResult::ZodiacApplied {
                                zodiac_kind,
                                yaku_name,
                                new_level,
                            };
                        }
                    }
                }
            }
        }
        ShopAction::BuyTalisman(idx) => {
            if idx < talisman_items.len() {
                let item = &talisman_items[idx];
                if !item.sold {
                    let price = item.price();
                    if let Consumable::Talisman(kind) = item.consumable {
                        let outcome =
                            engine.dispatch_shop(ShopCommand::BuyTalisman { kind, price });
                        if outcome.rejection.is_none() {
                            talisman_items.remove(idx);
                        }
                    }
                }
            }
        }
        ShopAction::SellConsumable(idx) => {
            let _ = engine.dispatch_shop(ShopCommand::SellConsumable { index: idx });
        }
        ShopAction::UseConsumable(idx) => {
            let outcome = engine.dispatch_shop(ShopCommand::UseConsumable { index: idx });
            if let ShopCommandData::ZodiacApplied {
                zodiac_kind,
                yaku_name,
                new_level,
            } = outcome.data
            {
                return ShopActionResult::ZodiacApplied {
                    zodiac_kind,
                    yaku_name,
                    new_level,
                };
            }
        }
        ShopAction::BuyPack(pack_idx) => {
            if let Some(pack) = pack_items.get_mut(pack_idx)
                && !pack.sold
            {
                let outcome = engine.dispatch_shop(ShopCommand::BuyPack {
                    kind: pack.kind,
                    price: pack.kind.shop_price(),
                });
                if outcome.rejection.is_none() {
                    pack.sold = true;
                }
                if let ShopCommandData::PackBought {
                    tiles,
                    pack_name,
                    pack_kind,
                } = outcome.data
                {
                    return ShopActionResult::PackCelebration(PackCelebration::new(
                        tiles, pack_name, pack_kind,
                    ));
                }
            }
            return ShopActionResult::None;
        }
    }
    ShopActionResult::None
}

fn live_shop_hit(
    hit: ShopHit,
    items: &[ShopItem],
    zodiac_items: &[ConsumableShopItem],
    talisman_items: &[ConsumableShopItem],
    pack_items: &[TilePackShopItem],
    shop: &ShopReadModel,
) -> Option<ShopHit> {
    let valid = match hit {
        ShopHit::Relic(i) => i < items.len() + shop.owned_relics.len(),
        ShopHit::Ribbon(i) => i < zodiac_items.len() + shop.owned_zodiacs.len(),
        ShopHit::Talisman(i) => i < talisman_items.len() + shop.owned_talismans.len(),
        ShopHit::Dish(id) => {
            if matches!(id, PICK_COIN_DISH | PICK_JOURNAL_BOOK) {
                true
            } else if let Some(idx) = tile_pack_index_from_pick(id) {
                pack_items.get(idx).is_some_and(|p| !p.sold)
            } else {
                false
            }
        }
        ShopHit::TilePack(id) => tile_pack_index_from_pick(id)
            .and_then(|idx| pack_items.get(idx))
            .is_some_and(|p| !p.sold),
    };
    valid.then_some(hit)
}

fn owned_ribbon_inventory_index(
    ribbon_idx: usize,
    zodiac_items: &[ConsumableShopItem],
    shop: &ShopReadModel,
) -> Option<usize> {
    if ribbon_idx < zodiac_items.len() {
        return None;
    }
    let oi = ribbon_idx - zodiac_items.len();
    shop.owned_zodiacs.get(oi).map(|item| item.inventory_index)
}

fn owned_talisman_inventory_index(
    talisman_idx: usize,
    talisman_items: &[ConsumableShopItem],
    shop: &ShopReadModel,
) -> Option<usize> {
    if talisman_idx < talisman_items.len() {
        return None;
    }
    let oi = talisman_idx - talisman_items.len();
    shop.owned_talismans
        .get(oi)
        .map(|item| item.inventory_index)
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

/// An owned item the player is currently dragging (keyboard/controller hold-A
/// or mouse press-and-drag) toward the sell tray. Stores enough information to
/// reconstruct the sell action once the item is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShopDragSource {
    /// An owned relic at flat-index `i` in the renderer relic list (i.e.
    /// `i >= n_for_sale_relics`). The value stored is the *owned* sub-index
    /// `i - n_for_sale`.
    OwnedRelic(usize),
    /// An owned consumable at index `inv_idx` in `run.consumables.items`.
    OwnedConsumable(usize),
}

impl ShopDragSource {
    /// Convert back to a `ShopAction::Sell*` action.
    fn sell_action(self) -> ShopAction {
        match self {
            Self::OwnedRelic(idx) => ShopAction::SellRelic(idx),
            Self::OwnedConsumable(idx) => ShopAction::SellConsumable(idx),
        }
    }
}

pub struct ShopScene {
    pub came_from_round: u32,
    mode: ShopMode,
    items: Vec<ShopItem>,
    zodiac_items: Vec<ConsumableShopItem>,
    talisman_items: Vec<ConsumableShopItem>,
    /// Tile packs for sale this shop visit (always up to `N_TILE_PACKS`).
    pack_items: Vec<TilePackShopItem>,
    /// Current reroll cost — starts at `REROLL_BASE_COST` and increases by
    /// `REROLL_COST_INCREMENT` each time the player rerolls this shop visit.
    reroll_cost: u32,
    pause_menu: PauseMenu,
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
    /// Per-bug orbit phase angles (radians). `BUG_COUNT` entries, each
    /// advances at a different speed each frame.
    bug_phases: [f32; BUG_COUNT],
    /// Per-relic glow start times. Populated when `relic_activations` is
    /// drained from the run state (e.g. Bonfire on relic sell, RitualBlade
    /// on destroy). Drives glow + wiggle on owned relics in the shop.
    relic_glow_starts: std::collections::HashMap<RelicId, Instant>,
    /// Normalized screen-relative positions for the shop scene.
    /// Loaded from JSON on construction; falls back to compiled defaults.
    pub positions: crate::ui::scene_layout::ShopPositions,
    /// Controller/keyboard "hold A to grab" drag source. Set when the player
    /// presses Confirm on an owned item; cleared on ConfirmRelease or Cancel.
    /// If set and the player releases Confirm while `SellTray` is focused,
    /// the item is sold. Also used to highlight the sell tray while held.
    held_item_drag: Option<ShopDragSource>,
    /// Mouse drag source. Set when the player presses the left mouse button
    /// over an owned item; cleared when the button is released. If released
    /// over the sell tray, the item is sold.
    mouse_drag: Option<ShopDragSource>,
    /// Per-`ShopHit` world-space overrides for the hover title/description
    /// plaque anchor. When a key is present, the scene uses the stored
    /// `(px, py, wz)` instead of the default layout-derived anchor for that
    /// item, letting authors hand-place plaques per object.
    pub hover_anchor_overrides: std::collections::HashMap<ShopHit, [f32; 3]>,
}

/// Click id for the `?` glossary badge in the shop HUD.
const SHOP_HELP_BADGE_ID: u32 = 0x9100;

// ── Bug swarm constants ───────────────────────────────────────────────────────
/// Number of 3D insects orbiting the lamp bulb (must be ≤ MAX_BUG_SLOTS).
const BUG_COUNT: usize = 6;
/// Per-bug: (orbit_radius_frac, orbit_z_offset_frac, orbit_speed_rad_per_sec, body_size_frac)
/// All fractions are relative to `lamp_h` (the lamp's Z scale).
const BUG_PARAMS: [(f32, f32, f32, f32); BUG_COUNT] = [
    (0.55, -0.10, 3.60, 0.90), // fast, close
    (0.80, -0.25, 2.25, 1.00), // medium
    (0.45, -0.05, 5.10, 0.75), // fast, tiny
    (0.90, -0.35, 1.65, 1.10), // slow, far
    (0.60, -0.20, 4.20, 0.85),
    (0.70, -0.15, 2.85, 0.95),
];
/// Click id for the catch-all 3D-hit dispatcher. When clicked, the shop's
/// update() routes the click based on `UpdateCtx::picked_shop_object`.
pub const SHOP_3D_HIT_ID: u32 = 0x9200;
/// Click id injected by `main.rs` when a mouse-drag that started on an owned
/// shop item is released over the sell tray. The shop's update() sells the
/// item referenced by `mouse_drag` when this fires.
pub const SHOP_DRAG_DROP_ID: u32 = 0x9700;
/// Click id for the Leave / advance 2D button (kept for focus-nav compat).
const SHOP_NEXT_ROUND_ID: u32 = 0x9300;
/// Click id for the Reroll 2D button (kept for focus-nav compat).
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
/// Base pick id for the for-sale tile packs on the shop shelf.
/// Two packs are offered, using ids `PICK_TILE_PACK_BASE` and
/// `PICK_TILE_PACK_BASE + 1`. Id `6` is reserved for `PICK_LEAVE_PROP`,
/// so only 2 ids are reserved here.
const PICK_TILE_PACK_BASE: u32 = 4;
/// Number of tile packs offered per shop visit.
const N_TILE_PACKS: usize = 2;
/// Pick id for the Leave action prop (counter right end).
const PICK_LEAVE_PROP: u32 = 6;
/// Pick id for the Reroll action prop (counter left end).
const PICK_REROLL_PROP: u32 = 7;
/// Pick id for the sell-return tray (inventory row far left).
const PICK_SELL_TRAY: u32 = 8;

/// Max for-sale relic slots on the kiosk (must match stock generation).
const KIOSK_RELIC_SLOTS: usize = 3;

/// True if `id` is one of the tile-pack pick ids.
fn is_tile_pack_pick(id: u32) -> bool {
    id >= PICK_TILE_PACK_BASE && id < PICK_TILE_PACK_BASE + N_TILE_PACKS as u32
}

/// Convert a tile-pack pick id to the index in `pack_items`.
fn tile_pack_index_from_pick(id: u32) -> Option<usize> {
    if is_tile_pack_pick(id) {
        Some((id - PICK_TILE_PACK_BASE) as usize)
    } else {
        None
    }
}

/// Pitch relic cuboids toward the camera (`rot_rx_rz_deg`).
/// The relic front cap is at local +Y; pitching past 90° tilts it to face -Y
/// (toward the camera). Camera is at (0, -0.72h, 0.34h); counter relics sit at
/// world_y ≈ +0.19h, so the relic-to-camera vector is roughly [0, -0.95, 0.31].
/// `arccos(-0.95) ≈ 162°` makes the face point directly at the camera.
const SHOP_RELIC_LEAN_COUNTER: f32 = 158.0;
/// Inventory row is closer to the camera (world_y ≈ -0.34h); relic-to-camera
/// vector ≈ [0, -0.74, 0.67] → `arccos(-0.74) ≈ 138°` for direct face-on.
const SHOP_RELIC_LEAN_INVENTORY: f32 = 138.0;

/// Generate randomized shop stock (relics + consumables) from the player's
/// unowned-relic pool. Shared between initial shop creation and rerolls.
fn generate_shop_stock(
    relics: &RelicState,
    available_relics: &[RelicId],
    extra_relics: usize,
) -> (
    Vec<ShopItem>,
    Vec<ConsumableShopItem>,
    Vec<ConsumableShopItem>,
    Vec<TilePackShopItem>,
) {
    let mut rng = rand::rng();

    const MAX_RIBBONS: usize = 4;
    let max_relics = KIOSK_RELIC_SLOTS;

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
        .filter(|d| {
            available_relics.contains(&d.id) && !relics.owns(d.id) && !shop_excluded.contains(&d.id)
        })
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

    let mut zodiac_pool: Vec<ZodiacKind> = ZodiacKind::all().to_vec();
    zodiac_pool.shuffle(&mut rng);
    let mut talisman_pool: Vec<TalismanKind> = TalismanKind::all().to_vec();
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

    // Always offer two unique tile packs.
    let pack_items: Vec<TilePackShopItem> = {
        let mut pack_pool: Vec<TilePackKind> = TilePackKind::all().to_vec();
        pack_pool.shuffle(&mut rng);
        pack_pool
            .into_iter()
            .take(N_TILE_PACKS)
            .map(|kind| TilePackShopItem { kind, sold: false })
            .collect()
    };

    (items, zodiac_items, talisman_items, pack_items)
}

fn tutorial_shop_stock() -> (
    Vec<ShopItem>,
    Vec<ConsumableShopItem>,
    Vec<ConsumableShopItem>,
    Vec<TilePackShopItem>,
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
        // Tutorial mode: two deterministic packs so the player can see
        // the side-by-side layout and still always encounter the canonical
        // tutorial pack (ScrollLibrary) in the first slot.
        vec![
            TilePackShopItem {
                kind: TilePackKind::ScrollLibrary,
                sold: false,
            },
            TilePackShopItem {
                kind: {
                    // Pick a second pack kind distinct from ScrollLibrary.
                    // Uses the first `all()` entry that differs.
                    let all = TilePackKind::all();
                    all.iter()
                        .copied()
                        .find(|&k| k != TilePackKind::ScrollLibrary)
                        .unwrap_or(TilePackKind::ScrollLibrary)
                },
                sold: false,
            },
        ],
    )
}

impl ShopScene {
    pub fn new(came_from_round: u32, run: &mut crate::game::run::RunState) -> Self {
        Self::new_with_mode(came_from_round, run, ShopMode::Standard)
    }

    pub fn new_tutorial(run: &mut crate::game::run::RunState) -> Self {
        Self::new_with_mode(GameEngine::current_run_number(run), run, ShopMode::Tutorial)
    }

    fn new_with_mode(
        came_from_round: u32,
        run: &mut crate::game::run::RunState,
        mode: ShopMode,
    ) -> Self {
        let shop = GameEngine::read_shop(run);
        let extra_relics = GameEngine::shop_extra_relic_stock(run);
        let (mut items, zodiac_items, talisman_items, pack_items) = if mode == ShopMode::Tutorial {
            tutorial_shop_stock()
        } else {
            generate_shop_stock(&shop.relic_state, &shop.available_relics, extra_relics)
        };

        // PatronGift: zero out one random relic's price.
        if mode == ShopMode::Standard && GameEngine::shop_has_patron_gift(run) && !items.is_empty()
        {
            use rand::prelude::IndexedMutRandom;
            let mut rng = rand::rng();
            if let Some(item) = items.choose_mut(&mut rng) {
                item.price = 0;
            }
        }

        let consumed_tags = GameEngine::consume_shop_tags(run);
        let reroll_cost = if mode == ShopMode::Tutorial {
            u32::MAX
        } else if consumed_tags.free_reroll {
            0
        } else {
            REROLL_BASE_COST
        };

        Self {
            came_from_round,
            mode,
            items,
            zodiac_items,
            talisman_items,
            pack_items,
            reroll_cost,
            pause_menu: PauseMenu::new(),
            focus: None,
            last_focus_rects: std::cell::RefCell::new(Vec::new()),
            pack_celebration: None,
            score_popups: ScorePopupSystem::new(),
            particles: ParticleSystem::new(),
            last_frame: Instant::now(),
            age_secs: 0.0,
            bug_phases: {
                // Spread initial phases evenly so bugs start distributed around the bulb.
                let mut phases = [0.0_f32; BUG_COUNT];
                for (i, p) in phases.iter_mut().enumerate() {
                    *p = i as f32 * std::f32::consts::TAU / BUG_COUNT as f32;
                }
                phases
            },
            relic_glow_starts: std::collections::HashMap::new(),
            positions: crate::ui::scene_layout::load_shop_positions(),
            held_item_drag: None,
            mouse_drag: None,
            hover_anchor_overrides: std::collections::HashMap::new(),
        }
    }

    fn continue_scene(&self, run: &mut crate::game::run::RunState) -> Scene {
        if self.mode == ShopMode::Tutorial {
            GameEngine::transition_to_onboarding_finale(run);
            Scene::Gameplay(super::gameplay::GameplayScene::with_pending_blind(
                crate::core::rules::BlindKind::Boss,
            ))
        } else {
            Scene::PickBlind(PickBlindScene::new())
        }
    }

    /// Apply a sell action and update `self.focus` so it stays on a
    /// neighbor of the sold item instead of snapping to `None`. Focus
    /// pointing at a different row is preserved; focus pointing at the
    /// sold row is rebased to the left neighbor (falling back to the
    /// sell tray when the row empties).
    fn apply_sell_action(
        &mut self,
        action: ShopAction,
        run: &mut crate::game::run::RunState,
        bus: &mut crate::game::event_bus::EventBus,
        cursor_pos: (f32, f32),
        overlay_request: &mut Option<super::OverlayRequest>,
    ) {
        let classified = classify_sell(action, &GameEngine::read_shop(run));
        let result = apply_shop_action(
            action,
            &mut self.items,
            &mut self.zodiac_items,
            &mut self.talisman_items,
            &mut self.pack_items,
            run,
            bus,
        );
        self.handle_shop_action_result(result, cursor_pos, bus, overlay_request);
        if let Some((row, sold_owned_idx)) = classified {
            let shop = GameEngine::read_shop(run);
            let owned_relics_after = shop.owned_relics.len();
            let owned_ribbons_after = shop.owned_zodiacs.len();
            let owned_talismans_after = shop.owned_talismans.len();
            self.focus = focus_after_sell(
                self.focus,
                row,
                sold_owned_idx,
                SellRowCounts {
                    n_for_sale_relics: self.items.len(),
                    n_for_sale_ribbons: self.zodiac_items.len(),
                    n_for_sale_talismans: self.talisman_items.len(),
                    owned_relics_after,
                    owned_ribbons_after,
                    owned_talismans_after,
                },
            );
        } else {
            self.focus = None;
        }
    }

    /// Route a `ShopActionResult` to the appropriate visual feedback.
    fn handle_shop_action_result(
        &mut self,
        result: ShopActionResult,
        _cursor_pos: (f32, f32),
        bus: &mut crate::game::event_bus::EventBus,
        overlay_request: &mut Option<super::OverlayRequest>,
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
                bus.push(crate::game::event_bus::GameEvent::ZodiacReveal);
                *overlay_request = Some(super::OverlayRequest::Push(Box::new(
                    Scene::ZodiacCelebration(super::ZodiacCelebrationScene::new(
                        zodiac_kind,
                        yaku_name,
                        new_level,
                    )),
                )));
            }
        }
    }

    /// Replace all unsold stock with fresh random items and bump the cost.
    fn reroll(&mut self, run: &mut crate::game::run::RunState) {
        if self.mode == ShopMode::Tutorial {
            return;
        }
        let mut bus = crate::game::event_bus::EventBus::default();
        let outcome = GameEngine::new(run, &mut bus).dispatch_shop(ShopCommand::RerollShop {
            cost: self.reroll_cost,
        });
        if outcome.rejection.is_some() {
            return;
        }
        self.reroll_cost += REROLL_COST_INCREMENT;
        let shop = GameEngine::read_shop(run);
        let (items, zodiac_items, talisman_items, pack_items) =
            generate_shop_stock(&shop.relic_state, &shop.available_relics, 0);
        self.items = items;
        self.zodiac_items = zodiac_items;
        self.talisman_items = talisman_items;
        self.pack_items = pack_items;
        self.focus = None;
    }

    /// Debug-only: reroll stock without deducting gold or incrementing cost.
    pub fn debug_reroll(&mut self, run: &crate::game::run::RunState) {
        let shop = GameEngine::read_shop(run);
        let (items, zodiac_items, talisman_items, pack_items) =
            generate_shop_stock(&shop.relic_state, &shop.available_relics, 0);
        self.items = items;
        self.zodiac_items = zodiac_items;
        self.talisman_items = talisman_items;
        self.pack_items = pack_items;
        self.focus = None;
    }

    /// Debug-only: open a random tile pack celebration without purchasing.
    pub fn debug_open_pack(&mut self, run: &mut crate::game::run::RunState) {
        use crate::core::tile_pack::TilePackKind;
        let mut rng = rand::rng();
        let all = TilePackKind::all();
        let kind = all[rand::RngExt::random_range(&mut rng, 0..all.len())];
        let tiles = GameEngine::debug_add_pack(run, kind);
        self.pack_celebration = Some(PackCelebration::new(tiles, kind.name(), kind));
    }
}

/// Spatial layout of the mountain kiosk shop (see `mahjuro_shop_world_ui_mockup-5.html`).
#[derive(Clone, Copy)]
struct ShopLayout {
    camera: CameraParams,
    /// Main counter volume (kiosk slab).
    counter_pixel_x: f32,
    counter_world_y: f32,
    counter_extents: [f32; 3],
    niche_centers_px: [(f32, f32, f32); KIOSK_RELIC_SLOTS],
    niche_count: usize,
    ribbon_anchors_px: [(f32, f32, f32); 8],
    ribbon_count: usize,
    ribbon_length: f32,
    ribbon_width: f32,
    talisman_anchors_px: [(f32, f32, f32); 4],
    talisman_anchor_count: usize,
    talisman_wall_width: f32,
    pack_centers_px: [(f32, f32, f32); N_TILE_PACKS],
    pack_extents: [f32; 3],
    relic_dish_center_px: (f32, f32, f32),
    relic_dish_extents: [f32; 3],
    ribbon_tray_center_px: (f32, f32, f32),
    ribbon_tray_extents: [f32; 3],
    talisman_tray_center_px: (f32, f32, f32),
    talisman_tray_extents: [f32; 3],
    /// Per-pendant nudge for the player's owned talismans on the
    /// `talisman_tray`. `(px, py, lift_world)` resolved from the
    /// `owned_talismans` placement at build time. Rotation from the
    /// placement is applied by the renderer via `apply_arrange_override`.
    owned_talisman_offset: (f32, f32, f32),
    coin_dish_center_px: (f32, f32, f32),
    coin_dish_extents: [f32; 3],
    owned_relic_count: usize,
    ribbon_owned_count: usize,
    talisman_owned_count: usize,
    consumable_length: f32,
    consumable_width: f32,
    /// Lamp band center for lighting `(px, py, lift)`.
    lamp_center_px: (f32, f32, f32),
    /// Pixels per millimeter (from the LayoutResult at build time).
    ppmm: f32,
}

impl ShopLayout {
    /// Convert millimeters to world units (same convention as `LayoutResult::mm`).
    fn mm(&self, n: f32) -> f32 {
        self.ppmm * n
    }
}

/// Per-category counts of for-sale and owned inventory used by
/// `ShopLayout::build` to size the shop's columns.
struct ShopInventoryCounts {
    n_for_sale: usize,
    n_for_sale_zodiacs: usize,
    n_for_sale_talismans: usize,
    n_owned_relics: usize,
    n_owned_zodiacs: usize,
    n_owned_talismans: usize,
}

impl ShopLayout {
    fn build(
        layout: &crate::ui::layout::LayoutResult,
        positions: &crate::ui::scene_layout::ShopPositions,
        counts: ShopInventoryCounts,
    ) -> Self {
        let ShopInventoryCounts {
            n_for_sale,
            n_for_sale_zodiacs,
            n_for_sale_talismans,
            n_owned_relics,
            n_owned_zodiacs,
            n_owned_talismans,
        } = counts;
        let w = layout.window_w;
        let h = layout.window_h;

        let camera = CameraParams {
            eye: [
                0.0,
                -h * positions.camera_eye_y_frac,
                h * positions.camera_eye_z_frac,
            ],
            target: [
                0.0,
                h * positions.camera_target_y_frac,
                h * positions.camera_target_z_frac,
            ],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 58.0,
        };

        // ── Counter slab ────────────────────────────────────────────────
        // DishExplicit: extents[0]=width(X), extents[1]=rim height(Z, added to lift),
        //               extents[2]=table depth(Y).
        let counter_extents = [w * 0.80, layout.mm(30.0), h * 0.17];
        let counter_pixel_x = positions.counter.nx * w;
        let counter_pixel_y = positions.counter.ny * h;
        // counter_world_y: world_y = pixel_y - h/2  (inverse of pixel_to_world which gives world_y = h/2 - pixel_y).
        // Draw sites use `counter_world_y + h * 0.5` to recover pixel_y.
        // ny=0.35 → pixel_y=0.35h → world_y = 0.35h - 0.5h = -0.15h  ✓ matches original.
        let counter_world_y = counter_pixel_y - h * 0.5;

        // ── For-sale stall positions ─────────────────────────────────────
        // Column pixel_x values from normalized nx fractions.
        let col_px_x: [f32; 4] = [
            positions.relics.nx * w,
            positions.packs.nx * w,
            positions.talismans.nx * w,
            positions.ribbons.nx * w,
        ];

        // Per-column ny / lift — each group has its own anchor.
        let relic_pixel_y = positions.relics.ny * h;
        let relic_wz = layout.mm(positions.relics.lift_mm);
        let pack_pixel_y = positions.packs.ny * h;
        let pack_wz = layout.mm(positions.packs.lift_mm);
        let talisman_pixel_y = positions.talismans.ny * h;
        let talisman_wz = layout.mm(positions.talismans.lift_mm);
        let ribbon_pixel_y = positions.ribbons.ny * h;
        let ribbon_wz = layout.mm(positions.ribbons.lift_mm);

        // Relics: up to 3 spread horizontally inside the relic column.
        let mut niche_centers_px = [(0.0, 0.0, 0.0); KIOSK_RELIC_SLOTS];
        let n_niches = n_for_sale.min(KIOSK_RELIC_SLOTS);
        let relic_spread = positions.relic_spread_nx * w;
        for (i, slot) in niche_centers_px.iter_mut().enumerate().take(n_niches) {
            let off = if n_niches <= 1 {
                0.0
            } else {
                (i as f32 - (n_niches as f32 - 1.0) * 0.5) * relic_spread
            };
            *slot = (col_px_x[0] + off, relic_pixel_y, relic_wz);
        }

        // Ribbons: hanging downward from anchor points at the counter edge.
        let ribbon_width = h * 0.055;
        let ribbon_length = ribbon_width * 2.0;
        let ribbon_pin_wz = ribbon_wz + ribbon_length;
        let mut ribbon_anchors_px = [(0.0, 0.0, 0.0); 8];
        let n_ribbons = n_for_sale_zodiacs.min(8);
        for (i, slot) in ribbon_anchors_px.iter_mut().enumerate().take(n_ribbons) {
            let off = (i as f32 - (n_ribbons as f32 - 1.0) * 0.5) * positions.ribbon_spread_nx * w;
            *slot = (col_px_x[3] + off, ribbon_pixel_y, ribbon_pin_wz);
        }

        // Talismans: standing upright after Rx(-90°) rotation.
        let talisman_wall_width = h * 0.072;
        let mut talisman_anchors_px = [(0.0, 0.0, 0.0); 4];
        let n_talisman_anchors = n_for_sale_talismans.min(4);
        for (i, slot) in talisman_anchors_px
            .iter_mut()
            .enumerate()
            .take(n_talisman_anchors)
        {
            let off = (i as f32 - (n_talisman_anchors as f32 - 1.0) * 0.5)
                * positions.talisman_spread_nx
                * w;
            *slot = (
                col_px_x[2] + off,
                talisman_pixel_y,
                talisman_wz + talisman_wall_width,
            );
        }

        // Tile pack: column 1, slightly taller lift. Width is derived from
        // the texture's canonical aspect so the cover art is never stretched;
        // thickness is a small fraction of the height for a card-pack feel.
        // Two tile packs: flank the `col_px_x[1]` center, spaced by
        // `pack_width + gap`. Pack extents are shared between both.
        let pack_height = h * 0.090;
        let pack_width = pack_height * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
        let pack_thickness = pack_height * 0.10;
        let pack_extents = [pack_width, pack_thickness, pack_height];
        let pack_gap = pack_width * 0.35;
        let pack_spacing = pack_width + pack_gap;
        let pack_z = pack_wz + layout.mm(10.0);
        let mut pack_centers_px = [(0.0, 0.0, 0.0); N_TILE_PACKS];
        for (i, slot) in pack_centers_px.iter_mut().enumerate() {
            let off = (i as f32 - (N_TILE_PACKS as f32 - 1.0) * 0.5) * pack_spacing;
            *slot = (col_px_x[1] + off, pack_pixel_y, pack_z);
        }

        // ── Bottom owned row ─────────────────────────────────────────────
        // shelf_ny was previously a single shared field; after migration each
        // shelf item owns its own `ny`. The four shelf items default to the
        // same value, so we read one of them as the authoritative shelf row.
        let shelf_pixel_y = positions.relic_dish.ny * h;
        let dish_rim = layout.mm(8.0);
        let tray_depth = h * 0.024;

        // Remap dish nx positions to the portion of the screen that is
        // actually within the camera frustum at the shelf's world depth.
        // pixel_to_world: world_y = h*0.5 - pixel_y
        let shelf_world_y = h * 0.5 - shelf_pixel_y;
        let (frust_world_x_min, frust_world_x_max) =
            camera.frustum_x_range_at(w, h, shelf_world_y, 0.0);
        // Convert frustum world-X bounds back to pixel-X (pixel_x = world_x + w*0.5).
        let frust_px_min = (frust_world_x_min + w * 0.5).max(0.0);
        let frust_px_max = (frust_world_x_max + w * 0.5).min(w);
        // Shrink by a margin so dish edges don't kiss the frustum boundary.
        let margin = w * 0.03;
        let vis_px_min = frust_px_min + margin;
        let vis_px_max = frust_px_max - margin;
        let vis_w = (vis_px_max - vis_px_min).max(1.0);
        // Map the configured nx values (which assume full screen width) into
        // the visible pixel range so every dish stays in view regardless of
        // camera angle or window aspect ratio.
        let remap_nx = |nx: f32| vis_px_min + nx * vis_w;

        let relic_dish_center_px = (remap_nx(positions.relic_dish.nx), shelf_pixel_y, 0.0);
        let relic_dish_extents = [vis_w * 0.14, dish_rim, tray_depth];
        let talisman_tray_center_px = (remap_nx(positions.talisman_tray.nx), shelf_pixel_y, 0.0);
        let talisman_tray_extents = [vis_w * 0.14, dish_rim, tray_depth];
        let ribbon_tray_center_px = (remap_nx(positions.ribbon_tray.nx), shelf_pixel_y, 0.0);
        let ribbon_tray_extents = [vis_w * 0.14, dish_rim, tray_depth];
        let coin_dish_center_px = (remap_nx(positions.coin_dish.nx), shelf_pixel_y, 0.0);
        let coin_dish_extents = [vis_w * 0.13, dish_rim, tray_depth];

        let consumable_width = layout.mm(9.0);
        let consumable_length = consumable_width * 1.5;

        // Owned-talisman nudge: convert the placement's window-fraction /
        // lift-mm values into the same (px, py, world) coordinates the draw
        // site uses so arrange-mode translation edits actually move the
        // rendered pendants.
        let ot = &positions.owned_talismans;
        let owned_talisman_offset = (ot.nx * w, ot.ny * h, layout.mm(ot.lift_mm));

        // Lamp: top-center, elevated above counter.
        let lamp_center_px = (
            positions.lamp.nx * w,
            positions.lamp.ny * h,
            layout.mm(positions.lamp.lift_mm),
        );

        Self {
            camera,
            counter_pixel_x,
            counter_world_y,
            counter_extents,
            niche_centers_px,
            niche_count: n_niches,
            ribbon_anchors_px,
            ribbon_count: n_ribbons,
            ribbon_length,
            ribbon_width,
            talisman_anchors_px,
            talisman_anchor_count: n_talisman_anchors,
            talisman_wall_width,
            pack_centers_px,
            pack_extents,
            relic_dish_center_px,
            relic_dish_extents,
            ribbon_tray_center_px,
            ribbon_tray_extents,
            talisman_tray_center_px,
            talisman_tray_extents,
            owned_talisman_offset,
            coin_dish_center_px,
            coin_dish_extents,
            owned_relic_count: n_owned_relics,
            ribbon_owned_count: n_owned_zodiacs,
            talisman_owned_count: n_owned_talismans,
            consumable_length,
            consumable_width,
            lamp_center_px,
            ppmm: layout.mm(1.0),
        }
    }

    fn owned_relic_pos(&self, idx: usize) -> (f32, f32, f32) {
        let n = self.owned_relic_count.max(1) as f32;
        let dish_w = self.relic_dish_extents[0] * 0.85;
        let start_x = self.relic_dish_center_px.0 - dish_w * 0.5 + (dish_w / n) * 0.5;
        let px = start_x + (dish_w / n) * idx as f32;
        let py = self.relic_dish_center_px.1;
        // lift_z: sit just above the rim (extents[1] is rim height in Z)
        let lift = self.relic_dish_extents[1] + 4.0;
        (px, py, lift)
    }

    fn owned_ribbon_pos(&self, idx: usize) -> (f32, f32, f32) {
        let n = self.ribbon_owned_count.max(1) as f32;
        let row_w = self.ribbon_tray_extents[0] * 0.85;
        let start_x = self.ribbon_tray_center_px.0 - row_w * 0.5 + (row_w / n) * 0.5;
        let px = start_x + (row_w / n) * idx as f32;
        let py = self.ribbon_tray_center_px.1;
        // lift_z: above tray rim, accounting for ribbon hanging half-length
        let lift = self.ribbon_tray_extents[1] + self.consumable_length * 0.5 + 6.0;
        (px, py, lift)
    }

    fn owned_talisman_pos(&self, idx: usize) -> (f32, f32, f32) {
        let n = self.talisman_owned_count.max(1) as f32;
        let row_w = self.talisman_tray_extents[0] * 0.85;
        let start_x = self.talisman_tray_center_px.0 - row_w * 0.5 + (row_w / n) * 0.5;
        let px = start_x + (row_w / n) * idx as f32;
        let py = self.talisman_tray_center_px.1;
        // lift_z: above tray rim
        let lift = self.talisman_tray_extents[1] + self.consumable_width * 0.5 + 6.0;
        let (ox, oy, olift) = self.owned_talisman_offset;
        (px + ox, py + oy, lift + olift)
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
/// so each cuboid reads as a distinct object. Sized for the kiosk relic stall.
fn relic_half_extents(id: RelicId, base: f32) -> [f32; 3] {
    let seed = (id as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
    let r0 = ((seed >> 8) & 0xFF) as f32 / 255.0;
    let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
    // Relic mesh: face disc lies in local XZ; local Y is the thickness axis.
    // half[0] = X half-extent (face width), half[1] = Y half-extent (thickness),
    // half[2] = Z half-extent (face height). Face is square to match 1:1 textures.
    let face = base * (0.65 + r0 * 0.45);
    [face, base * (0.08 + r2 * 0.04), face]
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
        Consumable::Talisman(t) => t.accent_color(),
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
) -> (Vec<Object3d>, Vec<Object3d>) {
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
    // Visual cap on the shop coin pile — covers any realistic gold
    // count without flooding the dish or burning shadow/draw time.
    const MAX_SHOP_COINS: usize = 64;
    let total_coins = (coin_gold as usize).min(MAX_SHOP_COINS);

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
        // Big bars first, then mini bars.
        let bar_specs: Vec<(usize, [f32; 3])> = std::iter::repeat_n((0, big_he), big_bars as usize)
            .chain(std::iter::repeat_n((1, mini_he), mini_bars as usize))
            .collect();
        for (spec_i, (_kind, he)) in bar_specs.iter().enumerate() {
            let row = spec_i / max_per_row;
            let col = spec_i % max_per_row;
            let cols_this_row = max_per_row.min(total_bars - row * max_per_row);
            let row_width = cols_this_row as f32 * he[0] * 2.5;
            let x_off = -row_width * 0.5 + he[0] * 1.25 + col as f32 * he[0] * 2.5;
            let world_y = dish_top_y + he[1] + row as f32 * (big_he[1] * 2.0 + 1.0);
            // Bars sit toward the back of the dish.
            let z_off = -dish_extents[2] * 0.25;
            // Gentle rotation drift for sparkle.
            let rot = 0.02 * (time * 0.5 + spec_i as f32 * 2.3).sin();
            bars.push(Object3d {
                pos: [dish_center_px.0 + x_off, dish_center_px.1 + z_off, world_y],
                extents: [he[0] * 2.0, he[1] * 2.0, he[2] * 2.0],
                rotation: rot_z_rad(rot),
                color: bar_color,
                kind: Object3dKind::Primitive {
                    shape: crate::render::primitive::MeshId::Cube,
                    material: crate::render::primitive::MaterialSpec::metal(),
                    pick_id: None,
                    shadow_caster: true,
                    silhouette: false,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: Some("shop.shelf.coin_dish"),
            });
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
                coins.push(Object3d {
                    pos: [dish_center_px.0 + x_off, dish_center_px.1 + z_off, world_y],
                    extents: [coin_radius * 2.0, coin_thickness, coin_radius * 2.0],
                    rotation: rot_z_rad(base_rot + sway),
                    color: gold_color,
                    kind: Object3dKind::Primitive {
                        shape: crate::render::primitive::MeshId::Cylinder,
                        material: crate::render::primitive::MaterialSpec::metal(),
                        pick_id: None,
                        shadow_caster: true,
                        silhouette: false,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some("shop.shelf.coin_dish"),
                });
                placed += 1;
            }
        }
    }

    (bars, coins)
}

fn shop_plaque_lines(scene: &ShopScene, shop: &ShopReadModel) -> (String, String) {
    let top = format!(
        "Shop  ·  Round {}  ·  Gold {}g",
        scene.came_from_round, shop.display_gold
    );
    (top, String::new())
}

impl SceneBehavior for ShopScene {
    fn pause_options_overlay(&self) -> Option<&super::options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    fn has_blocking_overlay(&self) -> bool {
        self.pause_menu.paused || self.pack_celebration.is_some()
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let shop = GameEngine::read_shop(ctx.run);
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.age_secs += dt;
        self.particles.update(dt);
        // Advance bug orbit phases.
        for (i, phase) in self.bug_phases.iter_mut().enumerate() {
            *phase = (*phase + BUG_PARAMS[i].2 * dt) % std::f32::consts::TAU;
        }
        self.score_popups.update(now);

        // Drain finished zodiac celebration → spawn popup + particles.
        if let Some((yaku_name, new_level)) = GameEngine::take_finished_zodiac_celebration(ctx.run)
        {
            let w = ctx.layout.window_w;
            let h = ctx.layout.window_h;
            let label = format!("{yaku_name} Lvl.{new_level}");
            let center = (w * 0.5, h * 0.45);
            self.score_popups.spawn(
                label,
                center,
                center,
                None,
                crate::core::scoring::StepKind::Gold,
                new_level as f32,
            );
            self.particles
                .emit(center.0, center.1, 24, [0.95, 0.78, 0.25, 1.0], 0.9);
        }

        // Drain relic activations and evict expired glows.
        for rid in GameEngine::drain_relic_activations(ctx.run) {
            self.relic_glow_starts.insert(rid, now);
            ctx.bus
                .push(crate::game::event_bus::GameEvent::RelicActivated(rid));
        }
        self.relic_glow_starts
            .retain(|_, start| now.saturating_duration_since(*start) < RELIC_GLOW_LIFETIME);

        // Help action opens the Meld Guide as an overlay.
        let mut open_guide = false;
        for &cid in ctx.button_clicks {
            if cid == SHOP_HELP_BADGE_ID {
                open_guide = true;
            }
        }
        for a in ctx.actions {
            if matches!(a, UiAction::Help) {
                open_guide = true;
            }
        }
        if open_guide {
            *ctx.overlay_request = Some(super::OverlayRequest::Push(Box::new(Scene::MeldGuide(
                super::meld_guide::MeldGuideScene::new(true),
            ))));
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

        // Pause menu handling.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            // Drain a meld guide request from the pause menu.
            if self.pause_menu.take_meld_guide_request() {
                *ctx.overlay_request = Some(super::OverlayRequest::Push(Box::new(
                    Scene::MeldGuide(super::meld_guide::MeldGuideScene::new(true)),
                )));
                return None;
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
                if self.focus.is_none() {
                    let seed = focus_rects
                        .iter()
                        .find_map(|(t, _)| matches!(t, ShopFocus::Relic(_)).then_some(*t))
                        .or_else(|| focus_rects.first().map(|(t, _)| *t));
                    if let Some(t) = seed {
                        self.focus = Some(t);
                    }
                    continue;
                }
                if let Some(rect) = current_focus_rect
                    && let Some(next) = pick_neighbor(rect, dir, &focus_rects)
                {
                    self.focus = Some(next);
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
                                self.apply_sell_action(
                                    ShopAction::SellRelic(owned_idx),
                                    ctx.run,
                                    ctx.bus,
                                    ctx.cursor_pos,
                                    ctx.overlay_request,
                                );
                            }
                            continue;
                        }
                        Some(ShopFocus::Ribbon(i)) => {
                            if let Some(inv_idx) =
                                owned_ribbon_inventory_index(i, &self.zodiac_items, &shop)
                            {
                                self.apply_sell_action(
                                    ShopAction::SellConsumable(inv_idx),
                                    ctx.run,
                                    ctx.bus,
                                    ctx.cursor_pos,
                                    ctx.overlay_request,
                                );
                            }
                            continue;
                        }
                        Some(ShopFocus::Talisman(i)) => {
                            if let Some(inv_idx) =
                                owned_talisman_inventory_index(i, &self.talisman_items, &shop)
                            {
                                self.apply_sell_action(
                                    ShopAction::SellConsumable(inv_idx),
                                    ctx.run,
                                    ctx.bus,
                                    ctx.cursor_pos,
                                    ctx.overlay_request,
                                );
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
                            &mut self.pack_items,
                            ctx.run,
                            ctx.bus,
                        );
                        // Update focus to follow the moved relic.
                        let shop = GameEngine::read_shop(ctx.run);
                        if matches!(a, UiAction::NavigateHudNext)
                            && owned_idx + 1 < shop.owned_relics.len()
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
            //
            // Controller/keyboard only: pressing Confirm on an owned item
            // starts a "hold to drag" — the item is grabbed but not yet sold.
            // The player navigates to the sell tray and releases Confirm to
            // complete the sale.  In cursor (mouse) mode the normal immediate
            // action fires instead, since mouse drag is handled separately via
            // SHOP_DRAG_DROP_ID.
            if matches!(a, UiAction::Confirm) {
                if ctx.input_mode != InputMode::Cursor
                    && let Some(src) = drag_source_from_focus(
                        self.focus,
                        self.items.len(),
                        &self.zodiac_items,
                        &self.talisman_items,
                        &shop,
                    )
                {
                    self.held_item_drag = Some(src);
                    continue;
                }
                if let Some(focus) = self.focus {
                    if matches!(focus, ShopFocus::NextRound) {
                        return Some(self.continue_scene(ctx.run));
                    }
                    if matches!(focus, ShopFocus::Reroll)
                        && self.mode == ShopMode::Standard
                        && shop.gold >= self.reroll_cost as i32
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
                            &shop,
                        ) {
                            let result = apply_shop_action(
                                action,
                                &mut self.items,
                                &mut self.zodiac_items,
                                &mut self.talisman_items,
                                &mut self.pack_items,
                                ctx.run,
                                ctx.bus,
                            );
                            self.handle_shop_action_result(
                                result,
                                ctx.cursor_pos,
                                ctx.bus,
                                ctx.overlay_request,
                            );
                        } else if matches!(hit, ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK) {
                            *ctx.overlay_request = Some(super::OverlayRequest::Push(Box::new(
                                super::Scene::YakuJournal(super::YakuJournalScene::new()),
                            )));
                            return None;
                        }
                    }
                }
                continue;
            }
            // ConfirmRelease: complete a controller/keyboard "hold to sell" drag
            // if the player released Confirm while the sell tray is focused.
            if matches!(a, UiAction::ConfirmRelease) {
                if let Some(drag) = self.held_item_drag.take()
                    && matches!(self.focus, Some(ShopFocus::SellTray))
                {
                    self.apply_sell_action(
                        drag.sell_action(),
                        ctx.run,
                        ctx.bus,
                        ctx.cursor_pos,
                        ctx.overlay_request,
                    );
                }
                continue;
            }
            // Cancel: clear focus and any active drag so the next directional
            // press re-seeds.
            if matches!(a, UiAction::Cancel) {
                self.held_item_drag = None;
                self.mouse_drag = None;
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
            if (SHOP_SELL_RELIC_BASE..SHOP_SELL_RELIC_BASE + 64).contains(&cid) {
                let idx = (cid - SHOP_SELL_RELIC_BASE) as usize;
                self.apply_sell_action(
                    ShopAction::SellRelic(idx),
                    ctx.run,
                    ctx.bus,
                    ctx.cursor_pos,
                    ctx.overlay_request,
                );
                return None;
            }
            if (SHOP_SELL_CONSUMABLE_BASE..SHOP_SELL_CONSUMABLE_BASE + 32).contains(&cid) {
                let idx = (cid - SHOP_SELL_CONSUMABLE_BASE) as usize;
                self.apply_sell_action(
                    ShopAction::SellConsumable(idx),
                    ctx.run,
                    ctx.bus,
                    ctx.cursor_pos,
                    ctx.overlay_request,
                );
                return None;
            }
            if cid == SHOP_NEXT_ROUND_ID {
                return Some(self.continue_scene(ctx.run));
            }
            if cid == SHOP_REROLL_ID
                && self.mode == ShopMode::Standard
                && shop.gold >= self.reroll_cost as i32
            {
                self.reroll(ctx.run);
                return None;
            }
        }

        // Mouse drag-to-sell drop: injected by main.rs when the mouse button is
        // released over the sell tray after a drag that started on an owned item.
        for &cid in ctx.button_clicks {
            if cid != SHOP_DRAG_DROP_ID {
                continue;
            }
            if let Some(drag) = self.mouse_drag.take() {
                self.apply_sell_action(
                    drag.sell_action(),
                    ctx.run,
                    ctx.bus,
                    ctx.cursor_pos,
                    ctx.overlay_request,
                );
            }
            return None;
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
            // Journal book intercept.
            if matches!(hit, ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK) {
                *ctx.overlay_request = Some(super::OverlayRequest::Push(Box::new(
                    super::Scene::YakuJournal(super::YakuJournalScene::new()),
                )));
                return None;
            }
            // Leave prop — advance to next scene.
            if matches!(hit, ShopHit::Dish(id) if id == PICK_LEAVE_PROP) {
                return Some(self.continue_scene(ctx.run));
            }
            // Reroll prop — restock shop if affordable.
            if matches!(hit, ShopHit::Dish(id) if id == PICK_REROLL_PROP) {
                if self.mode == ShopMode::Standard && shop.gold >= self.reroll_cost as i32 {
                    self.reroll(ctx.run);
                }
                return None;
            }
            // Sell tray — sell the dragged item (mouse_drag) if one is active,
            // otherwise fall back to selling whatever is focused.
            if matches!(hit, ShopHit::Dish(id) if id == PICK_SELL_TRAY) {
                let sell_action = self.mouse_drag.take().map(|d| d.sell_action()).or_else(|| {
                    focused_sell_action(
                        self.focus,
                        self.items.len(),
                        &self.zodiac_items,
                        &self.talisman_items,
                        &shop,
                    )
                });
                if let Some(action) = sell_action {
                    self.apply_sell_action(
                        action,
                        ctx.run,
                        ctx.bus,
                        ctx.cursor_pos,
                        ctx.overlay_request,
                    );
                }
                return None;
            }
            // For any other 3D hit: if it's an owned item with no action (relic
            // dish, owned zodiac going to UseConsumable, etc.) record it as a
            // potential mouse drag source so a subsequent sell-tray click can
            // sell it.  Clear the drag for unrelated hits.
            let drag_src = drag_source_from_hit(
                hit,
                self.items.len(),
                &self.zodiac_items,
                &self.talisman_items,
                &shop,
            );
            self.mouse_drag = drag_src;
            if let Some(action) = shop_action_for_hit(
                hit,
                &self.items,
                &self.zodiac_items,
                &self.talisman_items,
                &shop,
            ) {
                let result = apply_shop_action(
                    action,
                    &mut self.items,
                    &mut self.zodiac_items,
                    &mut self.talisman_items,
                    &mut self.pack_items,
                    ctx.run,
                    ctx.bus,
                );
                self.handle_shop_action_result(
                    result,
                    ctx.cursor_pos,
                    ctx.bus,
                    ctx.overlay_request,
                );
            }
            return None;
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;
        let shop = GameEngine::read_shop(ctx.run);
        let n_for_sale_zodiacs = self.zodiac_items.len();
        let n_for_sale_talismans = self.talisman_items.len();
        let n_owned_relics = shop.owned_relics.len();
        let n_owned_zodiacs = shop.owned_zodiacs.len();
        let n_owned_talismans = shop.owned_talismans.len();
        let layout = ShopLayout::build(
            ctx.layout,
            &self.positions,
            ShopInventoryCounts {
                n_for_sale: self.items.len(),
                n_for_sale_zodiacs,
                n_for_sale_talismans,
                n_owned_relics,
                n_owned_zodiacs,
                n_owned_talismans,
            },
        );

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        // Procedural mountain-haze wash. Additively composed onto the black
        // background, sits behind the 3D scene and above the volumetric
        // smoke curtain so the scene reads as "shop on a foggy mountain."
        frame.mountain_haze();
        frame.camera_override = Some(layout.camera);

        let (plaque_top_text, plaque_bot_text) = shop_plaque_lines(self, &shop);

        // ── Kiosk counter slab (thin wide dish — reads as a face-up surface) ─
        // center_pos is (pixel_x, pixel_y, lift_z); extents are (width_x, rim_z, depth_y).
        frame.object3d(Object3d {
            pos: [
                layout.counter_pixel_x,
                layout.counter_world_y + h * 0.5,
                layout.counter_extents[1] * 0.5,
            ],
            extents: layout.counter_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });

        // ── Foreground dishes (relic + talisman + ribbon trays + gold) ─
        frame.object3d(Object3d {
            pos: [
                layout.relic_dish_center_px.0,
                layout.relic_dish_center_px.1,
                layout.relic_dish_center_px.2 + layout.relic_dish_extents[1] * 0.5,
            ],
            extents: layout.relic_dish_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: Some(PICK_RELIC_DISH),
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.shelf.relic_dish"),
        });
        frame.object3d(Object3d {
            pos: [
                layout.talisman_tray_center_px.0,
                layout.talisman_tray_center_px.1,
                layout.talisman_tray_center_px.2 + layout.talisman_tray_extents[1] * 0.5,
            ],
            extents: layout.talisman_tray_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.shelf.talisman_tray"),
        });
        frame.object3d(Object3d {
            pos: [
                layout.ribbon_tray_center_px.0,
                layout.ribbon_tray_center_px.1,
                layout.ribbon_tray_center_px.2 + layout.ribbon_tray_extents[1] * 0.5,
            ],
            extents: layout.ribbon_tray_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.shelf.ribbon_tray"),
        });
        frame.object3d(Object3d {
            pos: [
                layout.coin_dish_center_px.0,
                layout.coin_dish_center_px.1,
                layout.coin_dish_center_px.2 + layout.coin_dish_extents[1] * 0.5,
            ],
            extents: layout.coin_dish_extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscRound,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: Some(PICK_COIN_DISH),
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });
        // Yaku Journal anchor — placed later (after `cam_rot` and
        // `hover` are in scope) as a wood action tablet. These
        // bindings stay here because downstream lighting code keys
        // point lights off `journal_cx/cy/cz`.
        let journal_cx = self.positions.book.nx * w;
        let journal_cy = self.positions.book.ny * h;
        let journal_cz = layout.mm(self.positions.book.lift_mm);

        // Tile packs — two flanking positions in column 2, on the counter.
        // Hidden while the pack-opening celebration is active: the celebration
        // draws its own large closeup pack centered on screen, and the 2D dim
        // quad can't depth-occlude the shelf packs behind it.
        if self.pack_celebration.is_none() {
            let ext = layout.pack_extents;
            let mut pack_objs: Vec<Object3d> = Vec::new();
            for (i, pack) in self.pack_items.iter().enumerate() {
                if i >= N_TILE_PACKS || pack.sold {
                    continue;
                }
                let (cx, cy, cz) = layout.pack_centers_px[i];
                pack_objs.push(Object3d {
                    // Center-lift so the pack sits on the counter (ext[2] is height).
                    pos: [cx, cy, cz + ext[2] * 0.5],
                    extents: ext,
                    rotation: glam::Mat4::IDENTITY,
                    color: pack.kind.foil_tint(),
                    kind: Object3dKind::Pack {
                        kind: pack.kind,
                        pick_id: Some(PICK_TILE_PACK_BASE + i as u32),
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
            if !pack_objs.is_empty() {
                frame.object3d_batch(pack_objs);
            }
        }

        // ── Relic batch: for-sale relics in column 1, then owned in tray.
        // The order matters: pick_shop_object
        // returns indices into a flat list, so we partition with the
        // for-sale slots first and the owned slots second.
        let mut relic_objects: Vec<Object3d> = Vec::new();
        let niche_base = layout.counter_extents[0] * 0.055;
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
            relic_objects.push(Object3d {
                pos: [px, py, wy + half[2]], // lift center by half face-height (local Z)
                extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                rotation: rot_rx_rz_deg(SHOP_RELIC_LEAN_COUNTER, 0.0),
                color: col,
                kind: Object3dKind::Relic {
                    relic_id: item.relic,
                    glow: 0.0,
                    silhouette: false,
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
        }
        let owned_base = layout.relic_dish_extents[0] * 0.15;
        for (i, &rid) in shop.owned_relics.iter().enumerate() {
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
            let wiggle = glam::Mat4::from_rotation_z(wiggle_deg.to_radians());
            relic_objects.push(Object3d {
                pos: [px, py, wy + half[2]], // lift center by half face-height (local Z)
                extents: [half[0] * 2.0, half[1] * 2.0, half[2] * 2.0],
                rotation: wiggle * rot_rx_rz_deg(SHOP_RELIC_LEAN_INVENTORY, 0.0),
                color: rarity_color(rarity),
                kind: Object3dKind::Relic {
                    relic_id: rid,
                    glow,
                    silhouette: false,
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
        }
        if !relic_objects.is_empty() {
            frame.object3d_batch(relic_objects);
        }

        // ── Consumable batches: zodiacs are silken ribbons (upper-right
        //    cabinet zone), talismans are jade octagonal tablets (lower-
        //    right cabinet zone, below the shelf divider). Each gets its
        //    own batch, pick path, and dedicated wall/tray positions.
        let mut consumable_objects: Vec<Object3d> = Vec::new();

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
            let w = layout.ribbon_width;
            consumable_objects.push(Object3d {
                pos: [ax, ay, awy],
                extents: [w, layout.ribbon_length, w * 0.15],
                rotation: rot_rz_ry_rx_deg(90.0, 0.0, 0.0),
                color: [1.0, 1.0, 1.0, col[3]],
                kind: Object3dKind::ZodiacRibbon {
                    kind: if let Consumable::Zodiac(z) = item.consumable {
                        Some(z)
                    } else {
                        None
                    },
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
        }

        // For-sale talismans: lower-right cabinet wall.
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
                consumable_objects.push(Object3d {
                    pos: [ax, ay, awy],
                    extents: [
                        layout.talisman_wall_width * 1.4,
                        layout.talisman_wall_width * 2.0,
                        layout.talisman_wall_width * 0.35,
                    ],
                    rotation: rot_rz_ry_rx_deg(-90.0, 0.0, 0.0),
                    color: col,
                    kind: Object3dKind::Talisman { kind: tk },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
        }

        // Owned consumables.
        for (z_ord, owned) in shop.owned_zodiacs.iter().enumerate() {
            if let Consumable::Zodiac(z) = owned.consumable {
                let (ax, ay, awy) = layout.owned_ribbon_pos(z_ord);
                let w = layout.consumable_width;
                consumable_objects.push(Object3d {
                    pos: [ax, ay, awy],
                    extents: [w, layout.consumable_length, w * 0.15],
                    rotation: rot_rz_ry_rx_deg(-90.0, 0.0, 0.0),
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::ZodiacRibbon { kind: Some(z) },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
        }
        for (t_ord, owned) in shop.owned_talismans.iter().enumerate() {
            if let Consumable::Talisman(tk) = owned.consumable {
                let (ax, ay, awy) = layout.owned_talisman_pos(t_ord);
                let w = layout.consumable_width;
                consumable_objects.push(Object3d {
                    pos: [ax, ay, awy - layout.consumable_length * 0.4],
                    extents: [w * 1.4, w * 2.0, w * 0.35],
                    rotation: rot_rz_ry_rx_deg(-90.0, 0.0, 0.0),
                    color: consumable_color(owned.consumable),
                    kind: Object3dKind::Talisman { kind: tk },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some("shop.shelf.owned_talismans"),
                });
            }
        }

        if !consumable_objects.is_empty() {
            frame.object3d_batch(consumable_objects);
        }

        // ── Gold display: bars + coin strings inside the coin dish ─────
        let (bars, coins) = coin_display_layout(
            shop.display_gold,
            layout.coin_dish_center_px,
            layout.coin_dish_extents,
            self.age_secs,
        );
        if !bars.is_empty() {
            frame.object3d_batch(bars);
        }
        if !coins.is_empty() {
            frame.object3d_batch(coins);
        }

        // ── Shop lamp ─────────────────────────────────────────────────
        // An overhead pendant lamp hanging above the counter.
        // Mesh is in world-space Z-up convention: no corrective rotation.
        // `pos` is the apex (top of shade / cord attachment point).
        // The shade rim hangs below (at apex_z + SHADE_RIM_Z * scale_z).
        // The bulb sits at LAMP_BULB_LOCAL_Z (negative, below apex).
        let lamp_w = h * 0.22;
        let lamp_h = h * 0.30;
        // Hanging point: center-screen (world_x=0), far back (world_y ≈ h*0.35,
        // which is lamp_center_px.ny=0.15 → pixel_y=h*0.15),
        // at world_z = h*0.52 (lamp_lift_h_frac).
        let lp = layout.lamp_center_px;
        let lamp_hang_z = lp.2; // apex z — lamp hangs downward from here
        // Flicker: layered sines at incommensurate rates plus an occasional
        // brownout dip sell a failing bulb on a foggy mountain. Held in
        // `lamp_flicker` so the shade glow, point lights, and god-rays all
        // pulse in lockstep.
        let tf = self.age_secs;
        let flick_fast = (tf * 37.3).sin() * 0.04 + (tf * 61.7).sin() * 0.025;
        let flick_slow = (tf * 4.1).sin() * 0.06;
        let brownout = {
            let d = (tf * 0.73).sin() * (tf * 1.19).sin();
            (d - 0.55).max(0.0) * 0.35
        };
        let lamp_flicker = (1.0 + flick_fast + flick_slow - brownout).clamp(0.55, 1.12);
        frame.object3d(Object3d {
            pos: [lp.0, lp.1, lamp_hang_z],
            extents: [lamp_w, lamp_h, lamp_w],
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::ShopLamp { glow: lamp_flicker },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });

        // ── 3D bugs orbiting the bulb ──────────────────────────────────
        // Bugs orbit in world XY around the bulb. Three wobble layers:
        //   1. Bob — sinusoidal Z drift, each bug at a different frequency.
        //   2. Radius drift — orbit radius breathes in/out (moths lunge at light).
        //   3. Bank — body rolls into the turn; extra pitch when bobbing.
        {
            let t_now = self.age_secs;
            let bulb_wx = lp.0 - w * 0.5;
            let bulb_wy = h * 0.5 - lp.1;
            let bulb_wz = lamp_hang_z + LAMP_BULB_LOCAL_Z * lamp_h;
            let bug_body_len = h * 0.022;

            // Wing flap parameters. Real moths beat their wings ~20-30 Hz
            // (hawkmoths even faster); we pick 25 Hz for a species-accurate
            // feel. At 60 fps a single flap cycle is only ~2.4 frames, so
            // the live wing would strobe on its own — the swept-fan blur
            // surrogate mesh (`build_bug_wing_blur_mesh`) is what actually
            // sells this as motion blur rather than aliasing: the live wing
            // fades near mid-stroke and the pre-swept fan takes over, then
            // vice-versa at the turnarounds. Amplitude 1.1 rad (~63°) sweeps
            // from below horizontal up past vertical, matching the way moths
            // clap their wings above the body between strokes. Per-bug phase
            // offsets keep the swarm from flapping in unison.
            let flap_hz: f32 = 25.0;
            let flap_amp: f32 = 1.1;

            // Sample a bug's full transform at `t_back` seconds in the past.
            // Kept parametric (rather than inlined at t_back = 0) so callers
            // that want to predict a bug's pose at a nearby moment — e.g.
            // shadow prediction, debug overlays — can reuse the same math.
            // Returns `(pos, extents, body_rot, flap_rad)` where `flap_rad`
            // is the wing angle at that moment (rotating about body +X).
            let sample_bug = |i: usize, t_back: f32| -> ([f32; 3], [f32; 3], glam::Mat4, f32) {
                let (r_frac, z_frac, speed, size_frac) = BUG_PARAMS[i];
                let fi = i as f32;
                let t = t_now - t_back;
                let phase = self.bug_phases[i] - speed * t_back;

                let bob_freq = 2.3 + fi * 0.71;
                let drift_freq = 1.1 + fi * 0.43;
                let pitch_freq = 3.7 + fi * 0.57;

                let bob = (t * bob_freq + fi * 1.3).sin() * lamp_h * 0.15;
                let r_nom = lamp_w * r_frac;
                let r_drift = (t * drift_freq + fi * 2.1).sin() * r_nom * 0.20;
                let bug_wz = bulb_wz + lamp_h * z_frac + bob;

                let local_z = (bug_wz - lamp_hang_z) / lamp_h;
                let min_r_local = shade_exclusion_radius(local_z);
                // Clear the shade by the body radius plus the inside wing's
                // span. The body is oriented tangent to the orbit, so wings
                // extend radially — the inner wing is the one that could
                // clip the shade, and it reaches ~1.13 units in local Y
                // (see moth_wing_outline) scaled by `size_frac`.
                let wing_half_span = 1.13 * size_frac * bug_body_len;
                let min_r_world =
                    min_r_local * (lamp_w / SHADE_RIM_R) + bug_body_len * 0.6 + wing_half_span;
                let orbit_r = (r_nom + r_drift).max(min_r_world);

                let bug_wx = bulb_wx + orbit_r * phase.cos();
                let bug_wy = bulb_wy + orbit_r * phase.sin();
                let bug_px = bug_wx + w * 0.5;
                let bug_py = h * 0.5 - bug_wy;
                let bug_sz = bug_body_len * size_frac;

                let tx = -phase.sin();
                let ty = phase.cos();
                let bank = std::f32::consts::FRAC_PI_4 * 0.5 + (t * 1.9 + fi * 0.8).sin() * 0.30;
                let pitch = (t * pitch_freq + fi * 0.5).sin() * 0.25;
                let yaw = glam::Mat4::from_cols(
                    glam::Vec4::new(tx, ty, 0.0, 0.0),
                    glam::Vec4::new(-ty, tx, 0.0, 0.0),
                    glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
                    glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
                );
                let rot =
                    yaw * glam::Mat4::from_rotation_x(bank) * glam::Mat4::from_rotation_y(pitch);
                // Flap angle at time `t`. Sine wave in radians, offset per
                // bug index so the swarm's wingbeats are phase-staggered.
                let flap = flap_amp * (t * flap_hz * std::f32::consts::TAU + fi * 1.3).sin();
                (
                    [bug_px, bug_py, bug_wz],
                    [bug_sz, bug_sz, bug_sz],
                    rot,
                    flap,
                )
            };

            // Live bugs — the ghost-trail system is gone; each bug now emits
            // two swept-fan blur-surrogate draws (L/R) alongside its crisp
            // live wings. The live wing fades where the real wing would blur
            // (mid-stroke) and the blur fan fades where the real wing would
            // read crisply (turnarounds), producing a coherent moth that
            // looks like a 1/60 s exposure.
            for i in 0..BUG_COUNT {
                let (pos, extents, rot, flap_rad) = sample_bug(i, 0.0);
                // Angular-speed factor in [0, 1]: 0 at the flap turnarounds
                // (where sin() peaks, cos() is 0) and 1 at mid-stroke. This
                // is |d/dt sin(w t)| / max, which reduces to |cos(w t)|.
                let fi = i as f32;
                let speed_factor = (t_now * flap_hz * std::f32::consts::TAU + fi * 1.3)
                    .cos()
                    .abs();
                let live_wing_alpha = 1.0 - 0.7 * speed_factor;
                let blur_alpha = 0.6 * speed_factor;
                frame.object3d(Object3d {
                    pos,
                    extents,
                    rotation: rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Bug {
                        slot: i,
                        flap_rad,
                        live_wing_alpha,
                        blur_alpha,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
                // Shadow casters: body + two wings as separate Gaussian
                // occluders so the lamp's god-ray shafts show recognisable
                // moth silhouettes instead of round blobs. Wing centres
                // are mesh-local (±Y, ~0.40 out from body) rotated through
                // the bug's orientation matrix so banking/pitch rotate
                // the silhouette with the live mesh.
                //
                // Wing flap: the occluder offset along ±Y shrinks as the
                // wings sweep toward vertical (edge-on to the lamp) and
                // swells back to full when the wings lie flat. That's
                // what makes the shaft silhouettes "flap" in sync with
                // the visible mesh — without this, the shafts would show
                // a static two-wing shape while the mesh moves.
                //
                // `pos` is `[pixel_x, pixel_y, world_z]`; occluder storage
                // expects pixel-space XY with world-space Z. Converting a
                // world-space offset back to pixel coords is `(+dx, -dy)`
                // because pixel-Y points down while world-Y points up (see
                // `pixel_to_world` in `render/world_space.rs`).
                // Body is a slender near-cylindrical mesh; Y/Z radius is
                // 0.11 in mesh-local units (see `build_bug_body_mesh`).
                let body_r = extents[0] * 0.24;
                let flap_c = flap_rad.cos();
                let flap_s = flap_rad.sin();
                // Edge-on wings cast almost no shadow; use cos(flap) to
                // collapse the Gaussian radius toward zero at ±90°.
                // Centroid of the moth-wing outline sits around Y ≈ 0.55
                // in mesh-local units; the Gaussian radius scales with the
                // wing area projected onto the shaft plane (cos(flap)).
                let wing_r = extents[0] * (0.40 + 0.32 * flap_c.abs());
                let wing_offset_y = 0.55_f32 * flap_c;
                let wing_offset_z = 0.55_f32 * flap_s;
                // Body occluder — compact core at the bug's centre.
                frame
                    .bug_occluders
                    .push(crate::render::draw_cmd::BugOccluder {
                        center_px: (pos[0], pos[1]),
                        lift: pos[2],
                        radius: body_r,
                        strength: 28.0,
                    });
                // Wing occluders — rotated offsets from the body centre.
                // Left wing flaps to +Z, right to −Z (mirror across body).
                let wing_locals = [
                    glam::Vec3::new(0.0, wing_offset_y, wing_offset_z),
                    glam::Vec3::new(0.0, -wing_offset_y, wing_offset_z),
                ];
                for wl in wing_locals {
                    let rotated = rot.transform_vector3(wl * extents[0]);
                    let cx_px = pos[0] + rotated.x;
                    let cy_px = pos[1] - rotated.y;
                    let cz = pos[2] + rotated.z;
                    frame
                        .bug_occluders
                        .push(crate::render::draw_cmd::BugOccluder {
                            center_px: (cx_px, cy_px),
                            lift: cz,
                            radius: wing_r,
                            strength: 22.0,
                        });
                }
            }
        }

        // ── Back-wall smoke curtain ────────────────────────────────────
        // A column of wind impulses along the back of the scene seeds a
        // billowing curtain of density that the volumetric smoke pass
        // renders as a slow, rolling sheet behind the stall items.
        // Phase offsets per-emitter break the row up so it reads as a
        // drape, not a uniform wall.
        //
        // Arrange-mode: `positions.smoke_curtain` nudges the row's center
        // (`nx` horizontal, `ny` vertical) and lifts it (`lift_mm`). The
        // curtain has no mesh, so it's cycle-only via Tab (not clickable).
        // Live preview folds the staged arrange delta in here because
        // wind gusts don't go through `apply_arrange_override`.
        //
        // Magnitudes (density, radius, velocity, lift, emitter count) are
        // driven by `ctx.shop_smoke_tuning`, live-editable from the
        // "Shop Smoke..." debug overlay.
        let t = self.age_secs;
        let smoke = ctx.shop_smoke_tuning;
        let n_emitters = smoke.emitter_count.max(1) as usize;
        let curtain_p = match ctx.arrange_preview.as_ref() {
            Some(prev) => prev.applied_to(
                crate::ui::scene_layout::SHOP_HIERARCHY,
                "shop.props.smoke_curtain",
                self.positions.smoke_curtain,
            ),
            None => self.positions.smoke_curtain,
        };
        let curtain_cx = curtain_p.nx * w;
        let back_pixel_y = curtain_p.ny * h;
        let span = w * 0.88;
        let curtain_lift = h * smoke.lift_fraction + layout.mm(curtain_p.lift_mm);
        for i in 0..n_emitters {
            let f = if n_emitters <= 1 {
                0.0
            } else {
                i as f32 / (n_emitters as f32 - 1.0) - 0.5
            };
            let cx = curtain_cx + f * span;
            let phase = i as f32 * 1.37;
            // Three overlapping sines at different rates give the curtain a
            // rolling, non-repeating billow instead of a uniform sway.
            let sway = (t * 0.45 + phase).sin();
            let roll = (t * 0.72 + phase * 0.6).sin();
            let billow = (t * 0.31 + phase * 0.9).sin();
            // Forward pulse breathes the sheet toward/away from camera.
            let breathe = 0.5 + 0.5 * (t * 0.38 + phase * 0.45).sin();
            frame.wind_gusts.push(crate::render::draw_cmd::WindGust {
                center_px: (
                    cx + sway * w * 0.045 + billow * w * 0.02,
                    back_pixel_y + billow * h * 0.03,
                ),
                lift: curtain_lift + roll * h * 0.09 + billow * h * 0.05,
                velocity: [
                    sway * 14.0 + billow * 6.0,
                    -6.0 - roll * 5.0,
                    smoke.forward_velocity_base
                        + breathe * smoke.forward_velocity_breathe_amp
                        + roll * 4.0,
                ],
                radius: h * (smoke.radius_base + smoke.radius_billow_amp * billow),
                density: smoke.density_base
                    + smoke.density_roll_amp * roll
                    + smoke.density_billow_amp * billow,
            });
        }

        // ── Smoky atmosphere ───────────────────────────────────────────
        // The fluid smoke pass renders curling volumetric haze across
        // the screen, depth-aware so it pools around the cabinet and
        // dishes. This is what sells the "shop in a backroom under a
        // dim lamp" mood — without it the scene reads as 3D objects on
        // a flat black UI page.
        frame.fluid_smoke();

        // ── Lighting: cold fluorescent lamp + purple rim ───────────────
        // LAMP_BULB_LOCAL_Z is negative (below apex), scaled by lamp_h.
        let lamp_bulb_pos = [lp.0, lp.1, lamp_hang_z + LAMP_BULB_LOCAL_Z * lamp_h];
        let mut point_lights: Vec<PointLight> = vec![
            // Cold fluorescent key — slightly greenish-white, typical of tube lighting.
            PointLight {
                pos: [
                    layout.lamp_center_px.0,
                    layout.lamp_center_px.1,
                    layout.lamp_center_px.2,
                ],
                radius: h * 1.15,
                color: [0.86, 0.96, 0.98],
                intensity: 2.15 * lamp_flicker,
            },
            // Cold fluorescent fill at the bulb itself.
            PointLight {
                pos: lamp_bulb_pos,
                radius: h * 1.30,
                color: [0.82, 0.94, 1.00],
                intensity: 2.60 * lamp_flicker,
            },
            // Purple rim highlight — offset beside the bulb to catch edges.
            PointLight {
                pos: [
                    lamp_bulb_pos[0],
                    lamp_bulb_pos[1],
                    lamp_bulb_pos[2] - h * 0.04,
                ],
                radius: h * 0.70,
                color: [0.72, 0.38, 1.00],
                intensity: 1.80 * lamp_flicker,
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
                    &self.pack_items,
                    &shop,
                )
            });
        // Pre-compute action prop state (needed for hover_info and 3D chrome rendering).
        let reroll_affordable =
            self.mode == ShopMode::Standard && shop.gold >= self.reroll_cost as i32;
        // Pre-compute hover item info for both the 3D plaque and the 2D description overlay.
        let n_for_sale_relics_hud = self.items.len().min(layout.niche_count);
        let hover_info: Option<(String, String, String, [f32; 4])> = hover.map(|hit| {
            let n_for_sale_zodiacs = self.zodiac_items.len();
            let n_for_sale_talismans = self.talisman_items.len();
            match hit {
                ShopHit::Relic(i) if i < n_for_sale_relics_hud => {
                    let item = &self.items[i];
                    let can_afford =
                        shop.gold >= item.price as i32 && !shop.relics_full && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        if shop.relics_full {
                            "Relics full".to_string()
                        } else {
                            format!("${} (have {}g)", item.price, shop.display_gold)
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
                    let oi = i - n_for_sale_relics_hud;
                    if oi < shop.owned_relics.len() {
                        let rid = shop.owned_relics[oi];
                        let defs = all_relic_defs();
                        let def = defs.iter().find(|d| d.id == rid);
                        let name = def
                            .map(|d| d.name.to_string())
                            .unwrap_or_else(|| "Relic".into());
                        let desc = relic_description_live(
                            rid,
                            &shop.relic_counters,
                            shop.total_score_earned,
                        );
                        let sell = relic_sell_price_live(rid, &shop.relic_counters);
                        (name, desc, format!("Sell {}g", sell), color::CHAMPAGNE)
                    } else {
                        (String::new(), String::new(), String::new(), color::SLATE)
                    }
                }
                ShopHit::Ribbon(i) if i < n_for_sale_zodiacs => {
                    let item = &self.zodiac_items[i];
                    let price = item.price();
                    let can_afford = shop.gold >= price as i32 && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        format!("${} (have {}g)", price, shop.display_gold)
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
                    let oi = i - n_for_sale_zodiacs;
                    if let Some(c) = shop.owned_zodiacs.get(oi).map(|item| item.consumable) {
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
                    let can_afford =
                        shop.gold >= price as i32 && !shop.consumables_full && !item.sold;
                    let cta = if item.sold {
                        "SOLD".to_string()
                    } else if !can_afford {
                        if shop.consumables_full {
                            "Inventory full".to_string()
                        } else {
                            format!("${} (have {}g)", price, shop.display_gold)
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
                    let oi = i - n_for_sale_talismans;
                    if let Some(c) = shop.owned_talismans.get(oi).map(|item| item.consumable) {
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
                    format!("{}g", shop.gold),
                    color::GOLD,
                ),
                ShopHit::Dish(id) if id == PICK_JOURNAL_BOOK => (
                    "Yaku Journal".to_string(),
                    "Levels, plays, and how to build every yaku".to_string(),
                    "Open".to_string(),
                    color::CHAMPAGNE,
                ),
                ShopHit::Dish(id) if id == PICK_LEAVE_PROP => (
                    if self.mode == ShopMode::Tutorial {
                        "Face Boss"
                    } else {
                        "Continue On"
                    }
                    .to_string(),
                    "Continue to the next round".to_string(),
                    String::new(),
                    color::CHAMPAGNE,
                ),
                ShopHit::Dish(id) if id == PICK_REROLL_PROP => (
                    "Restock".to_string(),
                    format!("Refresh shop for {}g", self.reroll_cost),
                    if reroll_affordable {
                        format!("{}g", self.reroll_cost)
                    } else {
                        format!("${} (have {}g)", self.reroll_cost, shop.display_gold)
                    },
                    if reroll_affordable {
                        color::GOLD
                    } else {
                        color::RUBY
                    },
                ),
                ShopHit::Dish(id) if id == PICK_SELL_TRAY => (
                    "Sell".to_string(),
                    "Focus an owned relic or consumable, then click here to sell it".to_string(),
                    String::new(),
                    color::CHAMPAGNE,
                ),
                ShopHit::Dish(id) if is_tile_pack_pick(id) => {
                    let idx = tile_pack_index_from_pick(id).unwrap_or(0);
                    if let Some(pack) = self.pack_items.get(idx) {
                        let price = pack.kind.shop_price();
                        let can_afford = shop.gold >= price as i32 && !pack.sold;
                        let cta = if pack.sold {
                            "SOLD".to_string()
                        } else if price == 0 {
                            "FREE".to_string()
                        } else if can_afford {
                            format!("Buy {}g", price)
                        } else {
                            format!("${} (have {}g)", price, shop.display_gold)
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
                ShopHit::TilePack(id) => {
                    let idx = tile_pack_index_from_pick(id).unwrap_or(0);
                    if let Some(pack) = self.pack_items.get(idx) {
                        let price = pack.kind.shop_price();
                        let can_afford = shop.gold >= price as i32 && !pack.sold;
                        let cta = if pack.sold {
                            "SOLD".to_string()
                        } else if price == 0 {
                            "FREE".to_string()
                        } else if can_afford {
                            format!("Buy {}g", price)
                        } else {
                            format!("${} (have {}g)", price, shop.display_gold)
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
            }
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
                let owned_target = hit_idx - n_for_sale;
                if owned_target < shop.owned_zodiacs.len() {
                    return Some(layout.owned_ribbon_pos(owned_target));
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
                let owned_target = hit_idx - n_for_sale;
                if owned_target < shop.owned_talismans.len() {
                    return Some(layout.owned_talisman_pos(owned_target));
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
                ShopHit::TilePack(id) => {
                    let idx = tile_pack_index_from_pick(id).unwrap_or(0);
                    let center = layout
                        .pack_centers_px
                        .get(idx)
                        .copied()
                        .unwrap_or(layout.pack_centers_px[0]);
                    point_lights.push(PointLight {
                        pos: [center.0, center.1 - 30.0, center.2 + 60.0],
                        radius: 180.0,
                        color: [1.00, 0.92, 0.70],
                        intensity: 3.20,
                    });
                }
            }
        }
        frame.point_lights = point_lights;

        // ── Hover item 3D anchor for plaque placement ───────────────────
        // Resolves the world-space top-face anchor of the currently hovered
        // item's AABB so the title plaque floats above it. Returned as
        // (pixel_x, pixel_y, world_z).
        let hover_item_pos: Option<(f32, f32, f32)> = hover.map(|hit| {
            if let Some(&[px, py, wz]) = self.hover_anchor_overrides.get(&hit) {
                return (px, py, wz);
            }
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            match hit {
                ShopHit::Relic(i) => {
                    if i < n_for_sale_relics {
                        let (px, py, wy) = layout.niche_centers_px[i];
                        let niche_base = layout.counter_extents[0] * 0.055;
                        let half = relic_half_extents(self.items[i].relic, niche_base);
                        (px, py, wy + half[2] * 2.0)
                    } else {
                        let oi = i - n_for_sale_relics;
                        if oi < n_owned_relics {
                            layout.owned_relic_pos(oi)
                        } else {
                            (w * 0.5, h * 0.5, 0.0)
                        }
                    }
                }
                ShopHit::Ribbon(i) => {
                    let n_for_sale = self.zodiac_items.len();
                    if i < n_for_sale {
                        if i < layout.ribbon_count {
                            layout.ribbon_anchors_px[i]
                        } else {
                            (w * 0.5, h * 0.5, 0.0)
                        }
                    } else {
                        let oi = i - n_for_sale;
                        layout.owned_ribbon_pos(oi)
                    }
                }
                ShopHit::Talisman(i) => {
                    let n_for_sale = self.talisman_items.len();
                    if i < n_for_sale {
                        if i < layout.talisman_anchor_count {
                            let (ax, ay, awy) = layout.talisman_anchors_px[i];
                            (ax, ay, awy + layout.talisman_wall_width)
                        } else {
                            (w * 0.5, h * 0.5, 0.0)
                        }
                    } else {
                        let oi = i - n_for_sale;
                        layout.owned_talisman_pos(oi)
                    }
                }
                ShopHit::TilePack(id) => {
                    let idx = tile_pack_index_from_pick(id).unwrap_or(0);
                    let center = layout
                        .pack_centers_px
                        .get(idx)
                        .copied()
                        .unwrap_or(layout.pack_centers_px[0]);
                    (center.0, center.1, center.2 + layout.pack_extents[2])
                }
                ShopHit::Dish(id) => {
                    if id == PICK_RELIC_DISH {
                        layout.relic_dish_center_px
                    } else if id == PICK_JOURNAL_BOOK {
                        (journal_cx, journal_cy, journal_cz)
                    } else if id == PICK_COIN_DISH {
                        layout.coin_dish_center_px
                    } else if id == PICK_REROLL_PROP {
                        (
                            self.positions.reroll_prop.nx * w,
                            layout.counter_world_y + h * 0.5,
                            layout.mm(self.positions.reroll_prop.lift_mm),
                        )
                    } else if id == PICK_LEAVE_PROP {
                        (
                            self.positions.leave_prop.nx * w,
                            layout.counter_world_y + h * 0.5,
                            layout.mm(self.positions.leave_prop.lift_mm),
                        )
                    } else if id == PICK_SELL_TRAY {
                        let vis_px_min = w * 0.25;
                        let vis_w2 = w * 0.5;
                        (
                            vis_px_min + self.positions.sell_tray.nx * vis_w2,
                            layout.relic_dish_center_px.1,
                            layout.mm(self.positions.sell_tray.lift_mm),
                        )
                    } else {
                        (w * 0.5, h * 0.5, 0.0)
                    }
                }
            }
        });

        // ── 3D shop chrome: Ofuda sign, info plaque, action props, sell tray ──
        let cam_rot = camera_facing_rotation(layout.camera.eye, layout.camera.target);

        // Path sign: Ofuda scroll. Arrange-mode placement contributes
        // additive deltas (position, lift, rotation) on top of the baked-in
        // -82° pitch and the counter-relative anchor.
        let ofuda_p = &self.positions.ofuda;
        frame.object3d(Object3d {
            pos: [
                w * 0.23 + ofuda_p.nx * w,
                layout.counter_world_y - h * 0.057 + h * 0.5 + ofuda_p.ny * h,
                layout.mm(147.66) + layout.mm(ofuda_p.lift_mm),
            ],
            extents: [w * 0.2, h * 0.12, layout.mm(3.0)],
            // Placement rx/ry/rz_deg are applied centrally by the renderer
            // via `committed_arrange_rotations`; keep only the baseline tilt
            // and the camera-facing yaw here.
            rotation: glam::Mat4::from_rotation_x((-82.0_f32).to_radians()) * cam_rot,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::Ofuda,
                material: crate::render::primitive::MaterialSpec::plain().with_decal(
                    crate::render::primitive::DecalSpec {
                        text: format!("{}\n{}", plaque_top_text, plaque_bot_text),
                        palette: crate::render::primitive::DecalPalette::ParchmentInk,
                        layout: crate::render::primitive::DecalLayout::TitleRule {
                            target_short_edge: crate::render::decal::OFUDA_DECAL_LONG_EDGE,
                        },
                    },
                ),
                pick_id: None,
                shadow_caster: false,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.props.ofuda"),
        });

        // Gold counter plaque — permanent slab on the counter near the coin
        // dish, showing the player's current gold at a glance.
        frame.object3d(Object3d {
            pos: [
                self.positions.coin_dish.nx * w,
                layout.counter_world_y + h * 0.5 - 60.0,
                layout.mm(self.positions.coin_dish.lift_mm) + layout.mm(10.0),
            ],
            extents: [w * 0.09, layout.mm(22.0), h * 0.045],
            rotation: cam_rot,
            color: [0.88, 0.78, 0.42, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::BeveledSlab,
                material: crate::render::primitive::MaterialSpec::lacquered_wood_flat().with_decal(
                    crate::render::primitive::plaque_decal(format!("Gold\n{}g", shop.display_gold)),
                ),
                pick_id: None,
                shadow_caster: false,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0xAC10,
            arrange_name: Some("shop.shelf.coin_dish"),
        });

        // Info plaque: title+CTA above the hovered item.
        //
        // Player-owned items live on the bottom-shelf trays (y≈0.84h), so the
        // for-sale layout — title above, description below — pushes the
        // description off the bottom of the screen. For owned items we draw a
        // single combined plaque (title / sell price / description) anchored
        // at its own arrange-mode placement so it can be positioned
        // independently from the for-sale hover plaques.
        let hover_is_owned = match hover {
            Some(ShopHit::Relic(i)) => i >= n_for_sale_relics_hud,
            Some(ShopHit::Ribbon(i)) => i >= self.zodiac_items.len(),
            Some(ShopHit::Talisman(i)) => i >= self.talisman_items.len(),
            _ => false,
        };
        if let Some((ref title, ref desc, ref cta, _)) = hover_info
            && !title.is_empty()
            && let Some((tpx, tpy, twz)) = hover_item_pos
        {
            let plaque_rot = glam::Mat4::from_rotation_x((-80.0_f32).to_radians()) * cam_rot;
            let plaque_z = layout.mm(4.0);

            // Clamp a plaque's pixel-X so its full width stays inside
            // the camera frustum at the plaque's world depth. Without
            // this, plaques anchored to edge items (far-left relics or
            // far-right ribbons) slide past the screen edge.
            let clamp_plaque_px = |center_px: f32, plaque_w: f32, py: f32, wz: f32| -> f32 {
                let world_y = h * 0.5 - py;
                let (fw_min, fw_max) = layout.camera.frustum_x_range_at(w, h, world_y, wz);
                let px_min = (fw_min + w * 0.5).max(0.0);
                let px_max = (fw_max + w * 0.5).min(w);
                let margin = w * 0.01;
                let lo = px_min + plaque_w * 0.5 + margin;
                let hi = px_max - plaque_w * 0.5 - margin;
                if hi <= lo {
                    (px_min + px_max) * 0.5
                } else {
                    center_px.clamp(lo, hi)
                }
            };

            if hover_is_owned {
                // Combined owned-item plaque: title / sell price /
                // description, stacked on one sign. Width sized like
                // the description plaque so long relic descriptions
                // don't shrink against the font clamp; height sized
                // to wrapped content.
                let owned_p = &self.positions.hover_owned_plaque;
                let plaque_w = w * 0.38;
                let text = if desc.is_empty() {
                    format!("{}\n{}", title, cta)
                } else {
                    format!("{}\n{}\n{}", title, cta, desc)
                };
                let font_px = (h * 0.022).max(14.0);
                let pad_frac = 0.1;
                let inner_w = plaque_w * (1.0 - 2.0 * pad_frac);
                let (line_count, line_h) =
                    measure_plaque_wrap(load_ui_font().as_ref(), &text, inner_w, font_px);
                let content_h = line_count as f32 * line_h;
                let plaque_h = (content_h / (1.0 - 2.0 * pad_frac)).max(h * 0.10);

                let py = tpy + owned_p.ny * h;
                let wz = (twz + h * 0.05 + layout.mm(owned_p.lift_mm)).max(0.0);
                let px = clamp_plaque_px(tpx + owned_p.nx * w, plaque_w, py, wz);
                frame.object3d(Object3d {
                    pos: [px, py, wz],
                    extents: [plaque_w, plaque_h, plaque_z],
                    rotation: plaque_rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Primitive {
                        shape: crate::render::primitive::MeshId::BeveledSlab,
                        material: crate::render::primitive::MaterialSpec::lacquered_wood_flat()
                            .with_decal(crate::render::primitive::plaque_decal(text)),
                        pick_id: None,
                        shadow_caster: false,
                        silhouette: false,
                    },
                    hover_target: 0.0,
                    anim_id: 0xAC0E,
                    arrange_name: Some("shop.hover.owned_plaque"),
                });
            } else {
                let plaque_w = w * 0.22;
                let plaque_h = h * 0.12;

                // Title plaque: anchored to the top face of the
                // item's AABB, floated up in screen-space and pulled
                // forward toward the camera. Arrange-mode placement
                // contributes additive deltas.
                let title_p = &self.positions.hover_title_plaque;
                let title_py = tpy - h * 0.28 + title_p.ny * h;
                let title_wz = twz + h * 0.14 + layout.mm(title_p.lift_mm);
                let title_px = clamp_plaque_px(tpx + title_p.nx * w, plaque_w, title_py, title_wz);
                frame.object3d(Object3d {
                    pos: [title_px, title_py, title_wz],
                    extents: [plaque_w, plaque_h, plaque_z],
                    rotation: plaque_rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Primitive {
                        shape: crate::render::primitive::MeshId::BeveledSlab,
                        material: crate::render::primitive::MaterialSpec::lacquered_wood_flat()
                            .with_decal(crate::render::primitive::plaque_decal(format!(
                                "{}\n{}",
                                title, cta
                            ))),
                        pick_id: None,
                        shadow_caster: false,
                        silhouette: false,
                    },
                    hover_target: 0.0,
                    anim_id: 0xAC00,
                    arrange_name: Some("shop.hover.title_plaque"),
                });

                // Description plaque: anchored below the item so the
                // player can read what the focused relic/ribbon/
                // talisman actually does.
                if !desc.is_empty() {
                    let desc_p = &self.positions.hover_desc_plaque;
                    // Wider + shorter than the title plaque.
                    let desc_w = w * 0.38;
                    let font_px = (h * 0.022).max(14.0);
                    let pad_frac = 0.1;
                    let inner_w = desc_w * (1.0 - 2.0 * pad_frac);
                    let (line_count, line_h) =
                        measure_plaque_wrap(load_ui_font().as_ref(), desc, inner_w, font_px);
                    let content_h = line_count as f32 * line_h;
                    let desc_h = (content_h / (1.0 - 2.0 * pad_frac)).max(h * 0.08);
                    let desc_py = tpy + h * 0.10 + desc_p.ny * h;
                    let desc_wz = (twz - h * 0.10 + layout.mm(desc_p.lift_mm)).max(0.0);
                    let desc_px = clamp_plaque_px(tpx + desc_p.nx * w, desc_w, desc_py, desc_wz);
                    frame.object3d(Object3d {
                        pos: [desc_px, desc_py, desc_wz],
                        extents: [desc_w, desc_h, plaque_z],
                        rotation: plaque_rot,
                        color: [1.0, 1.0, 1.0, 1.0],
                        kind: Object3dKind::Primitive {
                            shape: crate::render::primitive::MeshId::BeveledSlab,
                            material: crate::render::primitive::MaterialSpec::lacquered_wood_flat()
                                .with_decal(crate::render::primitive::plaque_decal(desc.clone())),
                            pick_id: None,
                            shadow_caster: false,
                            silhouette: false,
                        },
                        hover_target: 0.0,
                        anim_id: 0xAC0D,
                        arrange_name: Some("shop.hover.desc_plaque"),
                    });
                }
            }
        }

        // Reroll prop — left end of counter.
        let reroll_label = if self.mode == ShopMode::Tutorial {
            "Curated Stock".to_string()
        } else {
            format!("Restock {}g", self.reroll_cost)
        };
        let reroll_color = if reroll_affordable {
            [0.85, 0.78, 0.55, 1.0]
        } else {
            [0.45, 0.42, 0.35, 1.0]
        };
        let hover_is_reroll = matches!(hover, Some(ShopHit::Dish(id)) if id == PICK_REROLL_PROP);
        {
            use crate::render::primitive::{
                DecalLayout, DecalPalette, DecalSpec, MaterialSpec, MeshId,
            };
            let disabled = !reroll_affordable;
            let alpha = if disabled { 0.45 } else { reroll_color[3] };
            let color = [reroll_color[0], reroll_color[1], reroll_color[2], alpha];
            frame.object3d(Object3d {
                pos: [
                    self.positions.reroll_prop.nx * w,
                    layout.counter_world_y + h * 0.5,
                    layout.mm(self.positions.reroll_prop.lift_mm),
                ],
                extents: [w * 0.09, layout.mm(35.0), h * 0.065],
                rotation: cam_rot,
                color,
                kind: Object3dKind::Primitive {
                    shape: MeshId::ShopActionProp,
                    material: MaterialSpec {
                        kind: crate::render::lit_mesh::MaterialKind::Plain,
                        specular_strength: 0.4,
                        specular_power: 32.0,
                        decal: Some(DecalSpec {
                            text: reroll_label,
                            palette: DecalPalette::GoldGilded,
                            layout: DecalLayout::Fixed {
                                width: 512,
                                height: 192,
                            },
                        }),
                    },
                    pick_id: Some(PICK_REROLL_PROP),
                    shadow_caster: false,
                    silhouette: false,
                },
                hover_target: if hover_is_reroll { 1.0 } else { 0.0 },
                anim_id: 0xAC01,
                arrange_name: Some("shop.props.reroll_prop"),
            });
        }

        // Leave prop — right end of counter.
        let leave_label = if self.mode == ShopMode::Tutorial {
            "Face Boss"
        } else {
            "Continue On"
        };
        let hover_is_leave = matches!(hover, Some(ShopHit::Dish(id)) if id == PICK_LEAVE_PROP);
        {
            use crate::render::primitive::{
                DecalLayout, DecalPalette, DecalSpec, MaterialSpec, MeshId,
            };
            frame.object3d(Object3d {
                pos: [
                    self.positions.leave_prop.nx * w,
                    layout.counter_world_y + h * 0.5,
                    layout.mm(self.positions.leave_prop.lift_mm),
                ],
                extents: [w * 0.09, layout.mm(35.0), h * 0.065],
                rotation: cam_rot,
                color: [0.92, 0.88, 0.72, 1.0],
                kind: Object3dKind::Primitive {
                    shape: MeshId::ShopActionProp,
                    material: MaterialSpec {
                        kind: crate::render::lit_mesh::MaterialKind::Plain,
                        specular_strength: 0.4,
                        specular_power: 32.0,
                        decal: Some(DecalSpec {
                            text: leave_label.to_string(),
                            palette: DecalPalette::GoldGilded,
                            layout: DecalLayout::Fixed {
                                width: 512,
                                height: 192,
                            },
                        }),
                    },
                    pick_id: Some(PICK_LEAVE_PROP),
                    shadow_caster: false,
                    silhouette: false,
                },
                hover_target: if hover_is_leave { 1.0 } else { 0.0 },
                anim_id: 0xAC02,
                arrange_name: Some("shop.props.leave_prop"),
            });
        }

        // Yaku Journal — wood action tablet styled like gameplay's
        // action-bar journal button. Replaces the 3D book prop so the
        // journal affordance reads the same across scenes. Click
        // routes through `ShopHit::Dish(PICK_JOURNAL_BOOK)` via the
        // WoodTablet dispatch's `pick_id` hook.
        frame.object3d(Object3d {
            pos: [journal_cx, journal_cy, journal_cz],
            extents: [w * 0.06, layout.mm(16.0), h * 0.11],
            rotation: cam_rot,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::WoodTablet {
                label: "Journal".to_string(),
                pick_id: Some(PICK_JOURNAL_BOOK),
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("shop.props.journal"),
        });

        // Sell tray — bottom shelf row, accessible from all item types.
        // Highlight when an item is being dragged toward it or when a sellable
        // item is focused (keyboard/controller mode).
        let has_sellable_focus = focused_sell_action(
            self.focus,
            self.items.len(),
            &self.zodiac_items,
            &self.talisman_items,
            &shop,
        )
        .is_some();
        let has_active_drag = self.held_item_drag.is_some() || self.mouse_drag.is_some();
        let sell_tray_color = if has_sellable_focus || has_active_drag {
            [0.70, 0.90, 0.60, 1.0]
        } else {
            [0.45, 0.55, 0.45, 0.7]
        };
        let sell_tray_px_x = {
            let vis_px_min = w * 0.25;
            let vis_w = w * 0.5;
            vis_px_min + self.positions.sell_tray.nx * vis_w
        };
        frame.object3d(Object3d {
            pos: [
                sell_tray_px_x,
                layout.relic_dish_center_px.1,
                layout.mm(self.positions.sell_tray.lift_mm),
            ],
            extents: [w * 0.09, layout.mm(4.0), h * 0.065],
            rotation: glam::Mat4::IDENTITY,
            color: sell_tray_color,
            kind: Object3dKind::SellTray {
                pick_id: Some(PICK_SELL_TRAY),
            },
            hover_target: 1.0,
            anim_id: 0xAC03,
            arrange_name: Some("shop.props.sell_tray"),
        });

        // ── 2D HUD: tooltip + chrome buttons ───────────────────────────
        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut texts: Vec<TextLabel> = Vec::new();
        let mut buttons: Vec<ButtonDef> = Vec::new();
        let _ = ctx.layout.score_panel;

        // ── Tutorial shop banner ────────────────────────────────────────
        // When the tutorial is active and the lesson is shop-enabled, show
        // a hint banner at the top of the screen guiding the player through
        // the shop UI — mirroring the gameplay scene's tutorial overlay
        // style.
        if self.mode == ShopMode::Tutorial {
            let n_for_sale_relics = self.items.len().min(layout.niche_count);
            let n_for_sale_zodiacs = self.zodiac_items.len();
            let n_for_sale_talismans = self.talisman_items.len();
            let has_bought = !shop.owned_relics.is_empty()
                || !shop.owned_zodiacs.is_empty()
                || !shop.owned_talismans.is_empty()
                || shop.full_hand_level > 1
                || (!self.pack_items.is_empty() && self.pack_items.iter().any(|p| p.sold));
            let (flavor, hint) = if has_bought {
                (
                    "Your loadout is ready.",
                    "Try selling an owned item if you want to see the refund flow, or use LB / RB on owned relics to reorder them before you face The Iconoclast.",
                )
            } else if self.items.is_empty() {
                ("The kiosk is bare\u{2026}", "Press Leave to move on.")
            } else if let Some(hit) = hover {
                match hit {
                    ShopHit::Relic(i) if i < n_for_sale_relics => (
                        "Relics are permanent run upgrades.",
                        "The left stall sells passive relics. Read the tooltip, check the gold cost, and buy the one that best helps your scoring plan.",
                    ),
                    ShopHit::Relic(_) => (
                        "Owned relics live in the lower-left tray.",
                        "Hover a relic in the tray to review its effect. Use the Sell button or press LB / [ to cash it out if you want to pivot your build.",
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
                        "Hover an owned ribbon in the bottom ribbon tray and click Use to apply its yaku level-up before the next blind.",
                    ),
                    ShopHit::Talisman(_) => (
                        "Owned talismans can be sold back.",
                        "Hover an owned talisman in the bottom talisman tray to inspect it or sell it for gold if you need room.",
                    ),
                    ShopHit::Dish(id) if is_tile_pack_pick(id) => (
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
                    "Four stalls: relics, packs, talismans, ribbons. Hover anything to inspect it, then buy what helps before pressing Leave.",
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

        let scale = metrics::scene_scale(w, h, ui_scale);

        // ── Focus rect graph + brass focus ring ────────────────────────
        //
        // Build a single list of `(ShopFocus, screen_rect)` covering
        // every navigable shop element this frame, then stash it for
        // `update()` to consume next frame. Same one-frame-stale pattern
        // the gameplay scene uses.
        //
        // Source rects:
        //   - Relics: `projected_relic_rects` (in-cabinet then in-dish order)
        //   - Ribbons: `projected_ribbon_rects`
        //   - Talismans: `projected_talisman_rects`
        //   - Dishes (incl. Leave/Reroll/SellTray props): `aux_dish_rects` paired with pick id
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
            if r[2] > 1.0
                && r[3] > 1.0
                && r[0].is_finite()
                && r[1].is_finite()
                && let Some(id) = pid
            {
                if is_tile_pack_pick(*id) {
                    focus_rect_graph.push((ShopFocus::Pack(*id), *r));
                } else if *id == PICK_LEAVE_PROP {
                    focus_rect_graph.push((ShopFocus::NextRound, *r));
                } else if *id == PICK_REROLL_PROP {
                    focus_rect_graph.push((ShopFocus::Reroll, *r));
                } else if *id == PICK_SELL_TRAY {
                    focus_rect_graph.push((ShopFocus::SellTray, *r));
                } else if *id != PICK_RELIC_DISH {
                    focus_rect_graph.push((ShopFocus::Dish(*id), *r));
                }
            }
        }

        // Push the brass focus ring on top of the 2D HUD layer so it
        // sits above the cabinet wood and dishes. Skipped during pause
        // because the overlay's own buttons take focus.
        if !self.pause_menu.paused
            && let Some(target) = self.focus
        {
            let rect_lookup = focus_rect_graph
                .iter()
                .find_map(|(t, r)| (*t == target).then_some(*r));
            if let Some(rect) = rect_lookup {
                push_focus_ring(rect, scale, w, h, &mut quads);
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
        self.pause_menu.draw(
            crate::ui::layout::ViewportCtx {
                window_w: w,
                window_h: h,
                ui_scale,
            },
            scale,
            &mut quads,
            &mut texts,
            &mut buttons,
        );
        // Fullscreen click-blocker behind the pause menu's own buttons so
        // missed clicks become no-ops instead of falling through.
        if self.pause_menu.paused {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Push 2D layers onto the frame after all 3D content.
        frame.quads(quads);
        frame.texts(texts);

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
                    let box_w = box_h * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
                    let box_d = box_h * 0.10;
                    let t = celeb.started_at.elapsed().as_secs_f32();
                    // Calm, slow bob using incommensurate frequencies so
                    // the motion feels organic, not mechanical.
                    let bob_x = (t * 0.7).sin() * h * 0.008;
                    let bob_y = (t * 0.5).sin() * h * 0.006;
                    let bob_rx = (t * 0.6).sin() * 2.5; // degrees
                    let bob_ry = (t * 0.8).cos() * 3.0; // degrees
                    frame.object3d_batch(vec![Object3d {
                        pos: [
                            w * 0.5 + bob_x,
                            h * self.positions.celeb_pack_closeup.ny + bob_y,
                            layout.mm(self.positions.celeb_pack_closeup.lift_mm) + box_h * 0.5,
                        ],
                        extents: [box_w, box_d, box_h],
                        rotation: rot_ry_rx_deg(bob_rx, bob_ry),
                        color: celeb.pack_kind.foil_tint(),
                        kind: Object3dKind::Pack {
                            kind: celeb.pack_kind,
                            pick_id: None,
                        },
                        hover_target: 0.0,
                        anim_id: 0,
                        arrange_name: Some("shop.celebrations.pack_closeup"),
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
                    let row_x0 = (w - total_w) * 0.5 + w * self.positions.celeb_pack_reveal.nx;
                    let row_py = h * self.positions.celeb_pack_reveal.ny;
                    let row_lift = layout.mm(self.positions.celeb_pack_reveal.lift_mm);
                    let src_px = w * 0.5;
                    let src_py = h * self.positions.celeb_pack_closeup.ny;
                    let src_lift = row_lift + h * 0.15;
                    let row_rx = self.positions.celeb_pack_reveal.rx_deg.to_radians()
                        + 60.0_f32.to_radians();
                    let row_ry = self.positions.celeb_pack_reveal.ry_deg.to_radians();
                    let row_rz =
                        self.positions.celeb_pack_reveal.rz_deg.to_radians() + std::f32::consts::PI;

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
                            tile: celeb.tiles[i],
                            center_pos: [px, py, lift],
                            rotation: [row_rx, row_ry, row_rz],
                            scale,
                            size_px: tile_size,
                            brightness: 1.0,
                            selected: false,
                            hovered: false,
                            outline: false,
                            glow: false,
                            glow_color: None,
                            pick_id: None,
                        });
                    }

                    frame
                        .cmds
                        .push(crate::render::draw_cmd::DrawCmd::ShowcaseTileBatch(
                            placements,
                        ));
                    // Pack-reveal tiles render through `ShowcaseTileBatch`,
                    // which has no per-tile `arrange_name`, so committed
                    // placement values (read above as `nx/ny/lift_mm/rx/ry/rz`)
                    // are the only way to nudge them. Select
                    // `shop.celebrations.pack_reveal` from the arrange-mode
                    // hierarchy via Tab — there is no clickable mesh anchor.

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
        let popup_scale = w.min(h) / 1080.0;
        let glyph_placements = self.score_popups.placements(now, popup_scale);
        if !glyph_placements.is_empty() {
            frame.object3d_batch(glyph_placements);
        }
        for (rect, color) in self.particles.instances() {
            frame.quad(GpuInstance { rect, color });
        }

        frame.buttons = buttons;
        frame.window_title = format!(
            "Mahjuro — Shop (Round {}) — Gold: {}",
            self.came_from_round, shop.gold
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

    #[test]
    fn shop_only_rolls_unlocked_relics() {
        let relics = RelicState::default();
        let available_relics = vec![RelicId::PairPower];

        let (items, _, _, _) = generate_shop_stock(&relics, &available_relics, 1);

        assert!(!items.is_empty());
        assert!(items.iter().all(|item| item.relic == RelicId::PairPower));
    }
}
