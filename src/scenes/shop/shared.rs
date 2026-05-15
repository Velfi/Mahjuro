use super::*;
use crate::core::tile_pack::TilePackKind;
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

#[inline]
pub(crate) fn shop_focus_inspectable(f: ShopFocus) -> bool {
    matches!(
        f,
        ShopFocus::Relic(_) | ShopFocus::Ribbon(_) | ShopFocus::Talisman(_) | ShopFocus::Pack(_)
    )
}

/// Every shop element a controller / keyboard player can navigate to.
/// Mirrors the [`ShopHit`] flat-index scheme so the same action-dispatch
/// path that handles mouse clicks can also handle Confirm presses, plus
/// a synthetic `NextRound` variant for the 2D button at the bottom of
/// the screen (which has no 3D pick equivalent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShopFocus {
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
            ShopHit::Dish(id) => Self::Dish(id),
            ShopHit::TilePack(id) => Self::Pack(id),
            ShopHit::EnvSpawnSlot(_) | ShopHit::EnvInvSlot(_) | ShopHit::EnvConsumableOrd(_) => {
                unreachable!("resolve GLB env picks via live_shop_hit before ShopFocus::from_hit")
            }
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
        ShopHit::EnvSpawnSlot(_) | ShopHit::EnvInvSlot(_) | ShopHit::EnvConsumableOrd(_) => None,
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
    let relic_snapshot = run.relics.clone();
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
                    let price = item.price(&mode_snapshot, &relic_snapshot);
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
                    let price = item.price(&mode_snapshot, &relic_snapshot);
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
                    price: mode_snapshot.scale_shop_price(super::apply_merchants_eye_discount(
                        pack.kind.shop_price(),
                        &relic_snapshot,
                    )),
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
    pub(super) fn price(
        &self,
        mode: &crate::game::game_mode::GameMode,
        relics: &crate::core::relic::RelicState,
    ) -> u32 {
        let base = match self.consumable {
            Consumable::Zodiac(_) => ZodiacKind::shop_price(),
            Consumable::Talisman(t) => t.shop_price(),
        };
        mode.scale_shop_price(super::apply_merchants_eye_discount(base, relics))
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
) {
    if rect[2] <= 1.0 || rect[3] <= 1.0 || !rect[0].is_finite() || !rect[1].is_finite() {
        return;
    }
    let badge_font = typography::size(typography::MICRO, window_h).max(12.0);
    let badge_h = (badge_font * 1.55).max(18.0);
    let badge_w = (badge_font * 4.8).max(52.0);
    let badge_x = rect[0] + rect[2] - badge_w * 0.88;
    let badge_y = rect[1] - badge_h * 0.22;

    quads.push(GpuInstance {
        rect: [badge_x, badge_y, badge_w, badge_h],
        color: [color::BRASS[0], color::BRASS[1], color::BRASS[2], 0.95], user: 0});
    quads.push(GpuInstance {
        rect: [badge_x + 2.0, badge_y + 2.0, badge_w - 4.0, badge_h - 4.0],
        color: [
            color::WALNUT_DEEP[0],
            color::WALNUT_DEEP[1],
            color::WALNUT_DEEP[2],
            0.96,
        ], user: 0});
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
pub(crate) enum CelebPhase {
    /// Show the pack box up close; wait for player click to open.
    Closeup,
    /// Tiles flying out and arranging in a row.
    Reveal,
}

/// Tile-pack opening celebration overlay. Blocks shop input while active
/// and shows the acquired tiles flying out of the pack one by one, then
/// arranging themselves in a readable row.
pub(crate) struct PackCelebration {
    /// The tiles acquired from this pack (with enhancements already stamped).
    pub(crate) tiles: Vec<Tile>,
    /// Pack name for the header label.
    pub(crate) pack_name: &'static str,
    /// Which pack kind this is (for rendering the box texture).
    pub(crate) pack_kind: TilePackKind,
    /// Current phase of the celebration.
    pub(crate) phase: CelebPhase,
    /// Wall-clock time the *reveal* phase started (after closeup).
    pub(crate) started_at: Instant,
    /// Whether the player has dismissed the celebration (confirm/click).
    pub(crate) dismissed: bool,
    /// Number of tiles whose reveal sound has already been fired.
    pub(crate) revealed_count: usize,
    /// When true, headless warmup does not auto-skip from [`CelebPhase::Closeup`] to Reveal
    /// (used for screenshots that must show the pack mesh).
    pub(crate) headless_hold_pack_closeup: bool,
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

    pub(crate) fn new(tiles: Vec<Tile>, pack_name: &'static str, pack_kind: TilePackKind) -> Self {
        Self {
            tiles,
            pack_name,
            pack_kind,
            phase: CelebPhase::Closeup,
            // Will be reset when transitioning to Reveal.
            started_at: Instant::now(),
            dismissed: false,
            revealed_count: 0,
            headless_hold_pack_closeup: false,
        }
    }

    /// Total animation duration (all tiles landed + settle pause).
    pub(crate) fn total_duration(&self) -> f32 {
        let n = self.tiles.len().max(1) as f32;
        (n - 1.0) * Self::STAGGER + Self::TILE_FLY_SECS + Self::SETTLE_SECS
    }

    /// Elapsed seconds since the celebration started.
    pub(crate) fn elapsed(&self) -> f32 {
        Instant::now()
            .saturating_duration_since(self.started_at)
            .as_secs_f32()
    }

    /// True once all tiles have landed and the settle pause is over.
    pub(crate) fn fully_settled(&self) -> bool {
        self.elapsed() >= self.total_duration()
    }

    /// Per-tile animation progress: 0.0 = hasn't started, 1.0 = landed.
    pub(crate) fn tile_progress(&self, idx: usize) -> f32 {
        let t = self.elapsed() - idx as f32 * Self::STAGGER;
        (t / Self::TILE_FLY_SECS).clamp(0.0, 1.0)
    }

    /// Headless preset: reveal phase with every tile landed and prompt visible (tile row, no pack mesh).
    #[allow(dead_code)] // kept for optional reveal-phase captures / future CLI
    pub(crate) fn screenshot_reveal_settled(
        tiles: Vec<crate::core::tile::Tile>,
        pack_name: &'static str,
        pack_kind: crate::core::tile_pack::TilePackKind,
    ) -> Self {
        let mut s = Self::new(tiles, pack_name, pack_kind);
        s.phase = CelebPhase::Reveal;
        let dur = s.total_duration();
        s.started_at = Instant::now() - std::time::Duration::from_secs_f32(dur + 0.5);
        s.revealed_count = s.tiles.len();
        s
    }

    /// Headless screenshot: pack closeup (3D box visible). Holds Closeup across headless ticks.
    pub(crate) fn screenshot_pack_closeup_headless(
        tiles: Vec<crate::core::tile::Tile>,
        pack_name: &'static str,
        pack_kind: crate::core::tile_pack::TilePackKind,
    ) -> Self {
        let mut s = Self::new(tiles, pack_name, pack_kind);
        s.headless_hold_pack_closeup = true;
        s.started_at = Instant::now() - std::time::Duration::from_secs_f32(10.0);
        s
    }
}
