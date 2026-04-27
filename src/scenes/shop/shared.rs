use super::*;
use crate::scenes::meld_guide;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShopAction {
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
pub(super) enum ShopFocus {
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
pub(super) enum ShopMode {
    Standard,
    Tutorial,
}

impl ShopFocus {
    pub(super) fn from_hit(h: ShopHit) -> Self {
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
    pub(super) fn to_hit(self) -> Option<ShopHit> {
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
pub(super) fn shop_action_for_hit(
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
pub(super) fn focused_sell_action(
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
pub(super) enum SoldRow {
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

pub(super) fn focus_after_sell(
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
pub(super) fn classify_sell(action: ShopAction, shop: &ShopReadModel) -> Option<(SoldRow, usize)> {
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
pub(super) fn drag_source_from_hit(
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
pub(super) fn drag_source_from_focus(
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
pub(super) enum ShopActionResult {
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
pub(super) fn apply_shop_action(
    action: ShopAction,
    items: &mut Vec<ShopItem>,
    zodiac_items: &mut Vec<ConsumableShopItem>,
    talisman_items: &mut Vec<ConsumableShopItem>,
    pack_items: &mut [TilePackShopItem],
    run: &mut crate::game::run::RunState,
    bus: &mut crate::game::event_bus::EventBus,
) -> ShopActionResult {
    // Capture the shop's price multiplier off the mode before wrapping the
    // run in a GameEngine — the engine borrows mutably so we can't read
    // `run.mode` while a `ShopCommand` dispatches.
    let mode_snapshot = run.mode.clone();
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
                    let price = item.price(&mode_snapshot);
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
                    let price = item.price(&mode_snapshot);
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
                    price: mode_snapshot.scale_shop_price(pack.kind.shop_price()),
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

/// A purchasable relic in the shop.
pub(super) struct ShopItem {
    pub(super) relic: RelicId,
    pub(super) name: &'static str,
    pub(super) description: &'static str,
    pub(super) rarity: Rarity,
    pub(super) price: u32,
    pub(super) sold: bool,
}

impl ShopItem {
    pub(super) fn buy_label(&self) -> String {
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
pub(super) struct ConsumableShopItem {
    pub(super) consumable: Consumable,
    pub(super) sold: bool,
}

impl ConsumableShopItem {
    pub(super) fn price(&self, mode: &crate::game::game_mode::GameMode) -> u32 {
        let base = match self.consumable {
            Consumable::Zodiac(_) => ZodiacKind::shop_price(),
            Consumable::Talisman(t) => t.shop_price(),
        };
        mode.scale_shop_price(base)
    }
    pub(super) fn name(&self) -> String {
        self.consumable.name()
    }
    pub(super) fn description(&self) -> String {
        match self.consumable {
            Consumable::Zodiac(z) => {
                let yk = z.yaku();
                format!(
                    "Levels {} for the rest of the run (+0.5 mult, +20 chips per level). {}",
                    yk.name(),
                    meld_guide::yaku_shape_text(yk),
                )
            }
            Consumable::Talisman(t) => t.description().to_string(),
        }
    }
}

/// A purchasable tile pack in the shop.
pub(super) struct TilePackShopItem {
    pub(super) kind: TilePackKind,
    pub(super) sold: bool,
}

pub(super) fn push_free_badge(
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
pub(super) enum CelebPhase {
    /// Show the pack box up close; wait for player click to open.
    Closeup,
    /// Tiles flying out and arranging in a row.
    Reveal,
}

/// Tile-pack opening celebration overlay. Blocks shop input while active
/// and shows the acquired tiles flying out of the pack one by one, then
/// arranging themselves in a readable row.
pub(super) struct PackCelebration {
    /// The tiles acquired from this pack (with enhancements already stamped).
    pub(super) tiles: Vec<Tile>,
    /// Pack name for the header label.
    pub(super) pack_name: &'static str,
    /// Which pack kind this is (for rendering the box texture).
    pub(super) pack_kind: TilePackKind,
    /// Current phase of the celebration.
    pub(super) phase: CelebPhase,
    /// Wall-clock time the *reveal* phase started (after closeup).
    pub(super) started_at: Instant,
    /// Whether the player has dismissed the celebration (confirm/click).
    pub(super) dismissed: bool,
    /// Number of tiles whose reveal sound has already been fired.
    pub(super) revealed_count: usize,
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

    pub(super) fn new(tiles: Vec<Tile>, pack_name: &'static str, pack_kind: TilePackKind) -> Self {
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
    pub(super) fn total_duration(&self) -> f32 {
        let n = self.tiles.len().max(1) as f32;
        (n - 1.0) * Self::STAGGER + Self::TILE_FLY_SECS + Self::SETTLE_SECS
    }

    /// Elapsed seconds since the celebration started.
    pub(super) fn elapsed(&self) -> f32 {
        Instant::now()
            .saturating_duration_since(self.started_at)
            .as_secs_f32()
    }

    /// True once all tiles have landed and the settle pause is over.
    pub(super) fn fully_settled(&self) -> bool {
        self.elapsed() >= self.total_duration()
    }

    /// Per-tile animation progress: 0.0 = hasn't started, 1.0 = landed.
    pub(super) fn tile_progress(&self, idx: usize) -> f32 {
        let t = self.elapsed() - idx as f32 * Self::STAGGER;
        (t / Self::TILE_FLY_SECS).clamp(0.0, 1.0)
    }
}

/// An owned item the player is currently dragging (keyboard/controller hold-A
/// or mouse press-and-drag) toward the sell tray. Stores enough information to
/// reconstruct the sell action once the item is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShopDragSource {
    /// An owned relic at flat-index `i` in the renderer relic list (i.e.
    /// `i >= n_for_sale_relics`). The value stored is the *owned* sub-index
    /// `i - n_for_sale`.
    OwnedRelic(usize),
    /// An owned consumable at index `inv_idx` in `run.consumables.items`.
    OwnedConsumable(usize),
}

impl ShopDragSource {
    /// Convert back to a `ShopAction::Sell*` action.
    pub(super) fn sell_action(self) -> ShopAction {
        match self {
            Self::OwnedRelic(idx) => ShopAction::SellRelic(idx),
            Self::OwnedConsumable(idx) => ShopAction::SellConsumable(idx),
        }
    }
}
