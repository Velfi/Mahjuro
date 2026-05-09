use crate::core::tile_pack::TilePackKind;
use crate::scenes::{
    ShowcasePresenter, ShowcaseScene, TilePackPresenter, ZodiacPresenter,
};

use super::view::snap_focus_after_shop_purchase;
use super::*;
use crate::scenes::{GameplayScene, OverlayRequest};

/// Generate randomized shop stock (relics + consumables) from the player's
/// unowned-relic pool. Shared between initial shop creation and rerolls.
pub(super) fn generate_shop_stock(
    relics: &RelicState,
    available_relics: &[RelicId],
    extra_relics: usize,
    pool_extinction: crate::game::run::RelicShopPoolExtinction,
    mode: &crate::game::game_mode::GameMode,
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
    // duplication, run-wide burn swaps, etc. See
    // [`crate::game::run::relic_eligible_for_shop_stock`].
    let mut relic_pool: Vec<&_> = defs
        .iter()
        .filter(|d| {
            crate::game::run::relic_eligible_for_shop_stock(
                d.id,
                relics,
                available_relics,
                pool_extinction,
            )
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
            price: mode.scale_shop_price(relic_shop_price(d.id, relics)),
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

fn tutorial_shop_stock(
    mode: &crate::game::game_mode::GameMode,
) -> (
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
            price: mode.scale_shop_price(relic_shop_price(pair_power.id, &RelicState::default())),
            sold: false,
        }],
        vec![ConsumableShopItem {
            consumable: Consumable::Zodiac(ZodiacKind::Dragon),
            sold: false,
        }],
        vec![ConsumableShopItem {
            consumable: Consumable::Talisman(TalismanKind::Pearl),
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
    pub fn new(run: &mut crate::game::run::RunState) -> Self {
        Self::new_with_mode(run, ShopMode::Standard)
    }

    pub fn new_tutorial(run: &mut crate::game::run::RunState) -> Self {
        Self::new_with_mode(run, ShopMode::Tutorial)
    }

    fn new_with_mode(run: &mut crate::game::run::RunState, mode: ShopMode) -> Self {
        let shop = GameEngine::read_shop(run);
        let extra_relics = GameEngine::shop_extra_relic_stock(run);
        let stake = run.mode.stake;
        let (mut items, zodiac_items, talisman_items, pack_items) = if mode == ShopMode::Tutorial {
            tutorial_shop_stock(&run.mode)
        } else {
            generate_shop_stock(
                &shop.relic_state,
                &shop.available_relics,
                extra_relics,
                run.relic_shop_pool_extinction(),
                &run.mode,
            )
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
        // Reroll base cost is driven by the run's stake; tutorial and
        // free-reroll tags still override it.
        let reroll_cost = if mode == ShopMode::Tutorial {
            u32::MAX
        } else if consumed_tags.free_reroll {
            0
        } else {
            stake.reroll_base_cost()
        };

        Self {
            mode,
            items,
            zodiac_items,
            talisman_items,
            pack_items,
            reroll_cost,
            pause_menu: PauseMenu::new(),
            focus: None,
            last_focus_rects: std::cell::RefCell::new(Vec::new()),
            score_popups: ScorePopupSystem::new(),
            particles: ParticleSystem::new(),
            last_frame: Instant::now(),
            age_secs: 0.0,
            leave_bell_hover_anim: 0.0,
            journal_open_amount: 0.0,
            journal_open_target: 0.0,
            journal_open_lock: None,
            journal_transition: None,
            journal_transition_locked_at: None,
            journal_was_open: false,
            bug_phases: {
                let mut phases = [0.0_f32; BUG_COUNT];
                for (i, p) in phases.iter_mut().enumerate() {
                    *p = i as f32 * std::f32::consts::TAU / BUG_COUNT as f32;
                }
                phases
            },
            relic_glow_starts: std::collections::HashMap::new(),
            positions: crate::ui::scene_layout::load_shop_positions(),
            drawn_env_height_scale: std::cell::Cell::new(
                crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE,
            ),
            west_sell_hold_started: None,
        }
    }

    pub(super) fn continue_scene(&self, run: &mut crate::game::run::RunState) -> Scene {
        if self.mode == ShopMode::Tutorial {
            GameEngine::transition_to_onboarding_finale(run);
            Scene::Gameplay(GameplayScene::with_pending_blind(
                crate::core::rules::BlindKind::Boss,
            ))
        } else {
            Scene::PickBlind(PickBlindScene::new())
        }
    }

    /// Apply a purchase / use-consumable action; on success, move focus to the
    /// nearest remaining purchasable item, or the leave bell when nothing is
    /// left to buy.
    pub(super) fn apply_buy_action(
        &mut self,
        action: ShopAction,
        run: &mut crate::game::run::RunState,
        bus: &mut crate::game::event_bus::EventBus,
        cursor_pos: (f32, f32),
        overlay_request: &mut Option<OverlayRequest>,
        window_wh: (f32, f32),
    ) {
        self.west_sell_hold_started = None;
        let prev_focus = self.focus;
        let before = (
            self.items.len(),
            self.zodiac_items.len(),
            self.talisman_items.len(),
            self.pack_items.iter().filter(|p| p.sold).count(),
        );
        let result = apply_shop_action(
            action,
            &mut self.items,
            &mut self.zodiac_items,
            &mut self.talisman_items,
            &mut self.pack_items,
            run,
            bus,
        );
        let defer_focus_snap = matches!(
            &result,
            ShopActionResult::PackCelebration(_) | ShopActionResult::ZodiacApplied { .. }
        );
        let after = (
            self.items.len(),
            self.zodiac_items.len(),
            self.talisman_items.len(),
            self.pack_items.iter().filter(|p| p.sold).count(),
        );
        self.handle_shop_action_result(result, cursor_pos, bus, overlay_request, run);
        if before != after && !defer_focus_snap {
            snap_focus_after_shop_purchase(self, prev_focus, window_wh.0, window_wh.1, run);
        }
    }

    /// Apply a sell action; on success, move focus like [`Self::apply_buy_action`]
    /// (nearest purchasable, or leave when the shelf is cleared).
    pub(super) fn apply_sell_action(
        &mut self,
        action: ShopAction,
        run: &mut crate::game::run::RunState,
        bus: &mut crate::game::event_bus::EventBus,
        cursor_pos: (f32, f32),
        overlay_request: &mut Option<OverlayRequest>,
        window_wh: (f32, f32),
    ) {
        self.west_sell_hold_started = None;
        let prev_focus = self.focus;
        let shop_before = GameEngine::read_shop(run);
        let before_owned = (
            shop_before.owned_relics.len(),
            shop_before.owned_zodiacs.len(),
            shop_before.owned_talismans.len(),
        );
        let result = apply_shop_action(
            action,
            &mut self.items,
            &mut self.zodiac_items,
            &mut self.talisman_items,
            &mut self.pack_items,
            run,
            bus,
        );
        self.handle_shop_action_result(result, cursor_pos, bus, overlay_request, run);
        let shop_after = GameEngine::read_shop(run);
        let after_owned = (
            shop_after.owned_relics.len(),
            shop_after.owned_zodiacs.len(),
            shop_after.owned_talismans.len(),
        );
        if before_owned != after_owned {
            snap_focus_after_shop_purchase(self, prev_focus, window_wh.0, window_wh.1, run);
        }
    }

    /// Route a `ShopActionResult` to the appropriate visual feedback.
    pub(super) fn handle_shop_action_result(
        &mut self,
        result: ShopActionResult,
        _cursor_pos: (f32, f32),
        bus: &mut crate::game::event_bus::EventBus,
        overlay_request: &mut Option<OverlayRequest>,
        run: &crate::game::run::RunState,
    ) {
        match result {
            ShopActionResult::None => {}
            ShopActionResult::PackCelebration(celeb) => {
                let shop_rm = GameEngine::read_shop(run);
                let inventory = ShopInventoryCounts {
                    n_for_sale: self.items.len(),
                    n_for_sale_zodiacs: self.zodiac_items.len(),
                    n_for_sale_talismans: self.talisman_items.len(),
                    n_owned_relics: shop_rm.owned_relics.len(),
                };
                *overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Showcase(
                    ShowcaseScene::new(ShowcasePresenter::TilePack(TilePackPresenter::new(
                        celeb, inventory,
                    ))),
                ))));
            }
            ShopActionResult::ZodiacApplied {
                zodiac_kind,
                yaku_name,
                new_level,
            } => {
                bus.push(crate::game::event_bus::GameEvent::ZodiacReveal);
                *overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Showcase(
                    ShowcaseScene::new(ShowcasePresenter::Zodiac(ZodiacPresenter::new(
                        zodiac_kind,
                        yaku_name,
                        new_level,
                    ))),
                ))));
            }
        }
    }

    /// Replace all unsold stock with fresh random items and bump the cost.
    pub(super) fn reroll(&mut self, run: &mut crate::game::run::RunState) {
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
        self.west_sell_hold_started = None;
        self.reroll_cost += REROLL_COST_INCREMENT;
        let shop = GameEngine::read_shop(run);
        let (items, zodiac_items, talisman_items, pack_items) = generate_shop_stock(
            &shop.relic_state,
            &shop.available_relics,
            0,
            run.relic_shop_pool_extinction(),
            &run.mode,
        );
        self.items = items;
        self.zodiac_items = zodiac_items;
        self.talisman_items = talisman_items;
        self.pack_items = pack_items;
        self.focus = None;
    }

    /// Debug-only: reroll stock without deducting gold or incrementing cost.
    pub fn debug_reroll(&mut self, run: &crate::game::run::RunState) {
        let shop = GameEngine::read_shop(run);
        let (items, zodiac_items, talisman_items, pack_items) = generate_shop_stock(
            &shop.relic_state,
            &shop.available_relics,
            0,
            run.relic_shop_pool_extinction(),
            &run.mode,
        );
        self.items = items;
        self.zodiac_items = zodiac_items;
        self.talisman_items = talisman_items;
        self.pack_items = pack_items;
        self.focus = None;
    }
}
