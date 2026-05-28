use crate::core::tile_pack::TilePackKind;
use crate::scenes::{ShowcasePresenter, ShowcaseScene, TilePackPresenter, ZodiacPresenter};

use super::view::{default_shop_focus_for_stock, snap_focus_after_shop_purchase};
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
    run: &crate::game::run::RunState,
) -> (
    Vec<ShopItem>,
    Vec<ConsumableShopItem>,
    Vec<ConsumableShopItem>,
    Vec<TilePackShopItem>,
) {
    let mut rng = rand::rng();

    let crate::game::run::ShopOfferCounts {
        n_relics,
        n_zodiacs,
        n_talismans,
    } = crate::game::run::roll_shop_offer_counts(
        extra_relics,
        crate::game::run::KIOSK_RELIC_SLOTS,
        &mut rng,
    );

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

    let mut zodiac_pool = run.zodiac_spawn_pool();
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
                kind: TilePackKind::Manzu,
                sold: false,
            },
            TilePackShopItem {
                kind: {
                    // Pick a second pack kind distinct from ScrollLibrary.
                    // Uses the first `all()` entry that differs.
                    let all = TilePackKind::all();
                    all.iter()
                        .copied()
                        .find(|&k| k != TilePackKind::Manzu)
                        .unwrap_or(TilePackKind::Manzu)
                },
                sold: false,
            },
        ],
    )
}

impl ShopScene {
    pub fn new(
        run: &mut crate::game::run::RunState,
        progress: &crate::core::progression::PlayerProgress,
    ) -> Self {
        Self::new_with_mode(run, ShopMode::Standard, progress)
    }

    pub fn new_tutorial(run: &mut crate::game::run::RunState) -> Self {
        Self::new_with_mode(
            run,
            ShopMode::Tutorial,
            &crate::core::progression::PlayerProgress::new(),
        )
    }

    fn new_with_mode(
        run: &mut crate::game::run::RunState,
        mode: ShopMode,
        _progress: &crate::core::progression::PlayerProgress,
    ) -> Self {
        if mode != ShopMode::Tutorial {
            run.chronicle.note_shop_visit();
        }
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
                run,
            )
        };

        // PatronGift: zero out one random relic's price per stacked tag.
        if mode == ShopMode::Standard && GameEngine::shop_has_patron_gift(run) && !items.is_empty()
        {
            use rand::RngExt;

            let patron_gifts = run.tag_patron_gift;
            let mut rng = rand::rng();
            for _ in 0..patron_gifts {
                let unpaid: Vec<usize> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.price > 0)
                    .map(|(idx, _)| idx)
                    .collect();
                if unpaid.is_empty() {
                    break;
                }
                let idx = unpaid[rng.random_range(0..unpaid.len())];
                items[idx].price = 0;
            }
        }

        let consumed_tags = GameEngine::consume_shop_tags(run);
        let remaining_free_rerolls = consumed_tags.free_reroll;
        // Reroll base cost is driven by the run's stake; tutorial and
        // free-reroll tags still override it.
        let reroll_cost = if mode == ShopMode::Tutorial {
            u32::MAX
        } else if remaining_free_rerolls > 0 {
            0
        } else {
            stake.reroll_base_cost()
        };

        let focus = Some(default_shop_focus_for_stock(
            &items,
            &zodiac_items,
            &talisman_items,
            &pack_items,
        ));
        Self {
            mode,
            items,
            zodiac_items,
            talisman_items,
            pack_items,
            reroll_cost,
            remaining_free_rerolls,
            pause_menu: PauseMenu::new(),
            focus,
            last_focus_rects: std::cell::RefCell::new(Vec::new()),
            score_popups: ScorePopupSystem::new(),
            particles: ParticleSystem::new(),
            last_frame: Instant::now(),
            age_secs: 0.0,
            leave_bell_hover_anim: 0.0,
            relic_glow_starts: rustc_hash::FxHashMap::default(),
            drawn_room_gltf_height_scale: std::cell::Cell::new(
                crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE,
            ),
            inspect_dolly: std::cell::Cell::new(crate::scenes::object3d_inspect::InspectDolly {
                phase: 0.0,
                last_tick: Instant::now(),
            }),
            last_inspect_cam: std::cell::Cell::new(None),
            west_sell_hold_started: None,
            storeroom_orbit_yaw: 0.0,
            storeroom_orbit_pitch: 0.0,
            gltf_anims: crate::render::room_gltf_anim::GltfAnimPlaybackSet::default(),
            departing_stock: Vec::new(),
        }
    }

    fn shop_gltf_clip_duration(clip_name: &str) -> Option<f32> {
        crate::render::room_glb::with_shop_glb_cpu(|opt| {
            opt.and_then(|cpu| cpu.gltf_anim_library.clip_duration(clip_name))
        })
    }

    /// Start or resume a named glTF clip embedded in `shop.glb`.
    pub fn play_gltf_anim(&mut self, clip_name: &str, looping: bool) -> bool {
        let Some(duration) = Self::shop_gltf_clip_duration(clip_name) else {
            log::warn!(
                "play_gltf_anim({clip_name}): clip not loaded (rebuild after updating shop.glb)"
            );
            return false;
        };
        self.gltf_anims.play(clip_name, duration, looping);
        true
    }

    /// Toggle pause for a playing glTF clip. Returns `Some(paused_now)` when active.
    pub fn toggle_pause_gltf_anim(&mut self, clip_name: &str) -> Option<bool> {
        self.gltf_anims.toggle_pause(clip_name)
    }

    /// Restart a glTF clip from time 0.
    pub fn restart_gltf_anim(&mut self, clip_name: &str) -> bool {
        let Some(duration) = Self::shop_gltf_clip_duration(clip_name) else {
            log::warn!(
                "restart_gltf_anim({clip_name}): clip not loaded (rebuild after updating shop.glb)"
            );
            return false;
        };
        self.gltf_anims.restart(clip_name, duration);
        true
    }

    /// Stop a glTF clip and return to bind pose.
    pub fn stop_gltf_anim(&mut self, clip_name: &str) {
        self.gltf_anims.stop(clip_name);
    }

    /// Active glTF animation samples for the current frame.
    pub(crate) fn gltf_anim_samples(&self) -> Vec<(String, f32)> {
        self.gltf_anims.active_samples()
    }

    /// Current `eyeball_travel` playback time in seconds, if active.
    pub fn eyeball_travel_playback_sec(&self) -> Option<f32> {
        self.gltf_anims.playback_sec("eyeball_travel")
    }

    /// Debug: start `eyeball_travel` if available and not already running.
    /// Returns `true` when playback is active after this call.
    pub fn debug_start_eyeball_travel(&mut self) -> bool {
        self.play_gltf_anim("eyeball_travel", false)
    }

    /// Debug: toggle pause state for `eyeball_travel` playback.
    /// Returns `Some(paused_now)` when toggled, `None` when clip is not active.
    pub fn debug_toggle_pause_eyeball_travel(&mut self) -> Option<bool> {
        self.toggle_pause_gltf_anim("eyeball_travel")
    }

    /// Debug: restart `eyeball_travel` from time 0.
    /// Returns `true` when clip is available.
    pub fn debug_restart_eyeball_travel(&mut self) -> bool {
        self.restart_gltf_anim("eyeball_travel")
    }

    /// Whether storeroom dwell time should accumulate toward the eyeball milestone.
    pub(crate) fn counts_storeroom_dwell_time(&self) -> bool {
        self.mode == ShopMode::Standard && !self.pause_menu.paused
    }

    /// Play (or restart) `eyeball_travel` when a 15-minute storeroom milestone is reached.
    pub(crate) fn play_eyeball_travel_milestone(&mut self) {
        const CLIP: &str = "eyeball_travel";
        if self.gltf_anims.is_playing(CLIP) {
            let _ = self.restart_gltf_anim(CLIP);
        } else {
            let _ = self.play_gltf_anim(CLIP, false);
        }
    }

    pub(super) fn continue_scene(&self, run: &mut crate::game::run::RunState) -> Scene {
        if self.mode == ShopMode::Tutorial {
            GameEngine::transition_to_onboarding_finale(run);
            Scene::Gameplay(Box::new(GameplayScene::with_pending_chamber(
                crate::core::rules::ChamberKind::Ordeal,
            )))
        } else {
            Scene::PickChamber(PickChamberScene::new())
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
                let _ = run;
                *overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Showcase(
                    ShowcaseScene::new(ShowcasePresenter::TilePack(Box::new(
                        TilePackPresenter::new(celeb),
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
        if self.mode == ShopMode::Tutorial || self.restock_exit_active() {
            return;
        }
        let mut bus = crate::game::event_bus::EventBus::default();
        let outcome = GameEngine::new(run, &mut bus).dispatch_shop(ShopCommand::RerollShop {
            cost: self.reroll_cost,
        });
        if outcome.rejection.is_some() {
            return;
        }
        run.chronicle.note_reroll();
        self.west_sell_hold_started = None;
        match outcome.data {
            ShopCommandData::Rerolled {
                skip_cost_escalation: true,
                ..
            } => {
                // Free reroll (skip tag or I Got A Guy): spend one queued waiver.
                if self.remaining_free_rerolls > 0 {
                    self.remaining_free_rerolls -= 1;
                }
                self.reroll_cost = if self.remaining_free_rerolls > 0 {
                    0
                } else {
                    run.mode.stake.reroll_base_cost()
                };
            }
            _ => self.reroll_cost += REROLL_COST_INCREMENT,
        }
        let shop = GameEngine::read_shop(run);
        let pending = self.pending_shop_stock_from_run(&shop, run);
        self.begin_restock_exit(pending, std::time::Instant::now(), true);
    }

    /// Debug-only: reroll stock without deducting gold or incrementing cost.
    pub fn debug_reroll(&mut self, run: &crate::game::run::RunState) {
        let shop = GameEngine::read_shop(run);
        let pending = self.pending_shop_stock_from_run(&shop, run);
        self.begin_restock_exit(pending, std::time::Instant::now(), true);
    }

    fn pending_shop_stock_from_run(
        &self,
        shop: &ShopReadModel,
        run: &crate::game::run::RunState,
    ) -> super::restock_exit::PendingShopStock {
        let (items, zodiac_items, talisman_items, pack_items) = generate_shop_stock(
            &shop.relic_state,
            &shop.available_relics,
            0,
            run.relic_shop_pool_extinction(),
            &run.mode,
            run,
        );
        let focus = default_shop_focus_for_stock(
            &items,
            &zodiac_items,
            &talisman_items,
            &pack_items,
        );
        super::restock_exit::PendingShopStock {
            items,
            zodiac_items,
            talisman_items,
            pack_items,
            focus: Some(focus),
        }
    }
}
