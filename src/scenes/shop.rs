//! Shop scene — between rounds; player can buy relics with gold (`THEME.md` storeroom + `shop.glb`).
//!
//! **[`ShopScene`]** is the [`crate::scenes::Scene`] variant; rendering and hit layout live in the internal `view` module.

mod actions;
mod draw;
mod focus;
mod layout;
pub(crate) mod pack_celebration;
pub(crate) mod pick_ids;
mod restock_exit;
mod shared;
mod update;
mod view;

pub(crate) use self::view::render_shop_frame;

use crate::scenes::object3d_inspect::InspectDolly;

pub(crate) use self::pack_celebration::PackCelebration;
use mahjuro_render::draw_cmd::CameraParams;
pub(crate) use mahjuro_render::scene_glue::shop_celebration_camera;

pub(crate) fn sync_item_inspect_orbit_target(
    scene: &ShopScene,
    run: &crate::game::run::RunState,
    w: f32,
    h: f32,
    orbit: &mut crate::scenes::object3d_inspect::ItemInspectOrbitState,
) {
    view::shop_sync_item_inspect_orbit_target(scene, run, w, h, orbit);
}

use self::layout::*;
use self::shared::*;

pub(super) use self::pick_ids::{
    N_TILE_PACKS, PICK_COIN_DISH, PICK_JOURNAL_BOOK, PICK_LEAVE_PROP, PICK_RESTOCK_PROP,
    PICK_TILE_PACK_BASE,
};

use rand::seq::SliceRandom;

use std::time::{Duration, Instant};

use crate::core::consumable::Consumable;
use crate::core::relic::{
    Rarity, RelicId, RelicState, all_relic_defs, apply_merchants_eye_discount, relic_shop_price,
};
use crate::core::talisman::TalismanKind;
use crate::core::zodiac::ZodiacKind;
use crate::game::engine::{GameEngine, ShopCommand, ShopCommandData, ShopReadModel};
use crate::render::particles::ParticleSystem;
use crate::render::score_popups::ScorePopupSystem;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, ShopHit, TextAlign, TextLabel};
use crate::ui::focus_nav::{FocusDir, FocusNavState, RectFocusSession};
use crate::ui::input::{InputMode, UiAction};
use super::pause_menu::PauseMenu;
pub(crate) use super::{Scene, SceneIntent, SceneTransition, UpdateCtx};

pub struct ShopScene {
    mode: ShopMode,
    items: Vec<ShopItem>,
    zodiac_items: Vec<ConsumableShopItem>,
    talisman_items: Vec<ConsumableShopItem>,
    /// Tile packs for sale this shop visit (always up to `N_TILE_PACKS`).
    pack_items: Vec<TilePackShopItem>,
    /// Paid restocks already used this shop visit (drives [`Self::restock_cost`]).
    paid_restocks_this_visit: u32,
    /// Temptation free restocks still owed this visit (consumed one at a time).
    remaining_free_restocks: u32,
    pause_menu: PauseMenu,
    /// Currently focused shop element. Starts on the first for-sale shelf item
    /// (or the leave bell when the shelf is empty). In cursor mode, hover follows
    /// the 3D pick when present; otherwise screen rects (shelf slots + HUD buttons).
    focus: Option<ShopFocus>,
    focus_nav: FocusNavState<ShopFocus>,
    /// Focus rect graph from the previous draw frame (stock + chrome for spatial nav).
    focus_session: RectFocusSession<ShopFocus>,
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
    /// Eased 0..1 while the leave bell is hovered or focus-selected — drives a
    /// subtle wobble on the bell mesh in `draw_frame`.
    leave_bell_hover_anim: f32,
    /// Per-relic glow start times. Populated when `relic_activations` is
    /// drained from the run state (e.g. Bonfire on relic sell). Drives glow +
    /// wiggle on owned relics in the shop.
    relic_glow_starts: rustc_hash::FxHashMap<RelicId, Instant>,
    /// Last `DrawCtx::room_gltf_height_scale` from `draw_frame` (updated each draw). Used when building focus rects from `update()` so marker math matches the GPU pass (possibly one frame behind).
    drawn_room_gltf_height_scale: std::cell::Cell<f32>,
    /// Eased blend between storeroom base camera and orbit inspect ([`crate::scenes::object3d_inspect::tick_inspect_dolly`]).
    inspect_dolly: std::cell::Cell<InspectDolly>,
    /// Last orbit inspect camera — used to ease out after the showcase overlay pops.
    last_inspect_cam: std::cell::Cell<Option<CameraParams>>,
    /// West-face hold-to-sell (gamepad West / **Q** / mouse hold on owned stock).
    west_sell_hold_started: Option<std::time::Instant>,
    /// Confirm hold-to-buy (gamepad South / Enter / Space). Cursor mode uses click-to-buy instead.
    confirm_buy_hold_started: Option<std::time::Instant>,
    /// One-shot onboarding hint after releasing a hold too early.
    hold_tooltip: Option<HoldTooltipState>,
    /// LMB-drag turntable on the storeroom camera (radians, applied around [`CameraParams::target`]).
    storeroom_orbit_yaw: f32,
    storeroom_orbit_pitch: f32,
    /// glTF node TRS clip playbacks for embedded room meshes (e.g. `ArmSwing`, `eyeball_travel`).
    gltf_anims: crate::render::room_gltf_anim::GltfAnimPlaybackSet,
    /// For-sale meshes still falling after restock; culled once off-screen.
    departing_stock: Vec<restock_exit::ShopDepartingBatch>,
    /// When the current for-sale row should play the enter scale pop.
    restock_enter_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug)]
struct HoldTooltipState {
    focus: ShopFocus,
    until: Instant,
}

/// Click id for the `?` glossary badge in the shop HUD.
const SHOP_HELP_BADGE_ID: u32 = 0x9100;

/// Click id for the catch-all 3D-hit dispatcher. When clicked, the shop's
/// update() routes the click based on `UpdateCtx::picked_shop_object`.
pub const SHOP_3D_HIT_ID: u32 = 0x9200;
/// Cursor-mode preview hint icon while item inspect is open.
pub const SHOP_INSPECT_PREVIEW_ID: u32 = 0x9201;
/// How long a relic glow + wiggle lasts after activation.
const RELIC_GLOW_LIFETIME: std::time::Duration = std::time::Duration::from_millis(900);
const HOLD_TOOLTIP_DURATION: Duration = Duration::from_millis(1600);
/// Pitch relic cuboids toward the camera ([`crate::render::table_transform::rot_fixed_axes_deg`]).
/// The relic front cap is at local +Y; pitching past 90° tilts it to face -Y
/// (toward the camera). Camera is at (0, -0.72h, 0.34h); counter relics sit at
/// world_y ≈ +0.19h, so the relic-to-camera vector is roughly [0, -0.95, 0.31].
/// `arccos(-0.95) ≈ 162°` makes the face point directly at the camera.
const SHOP_RELIC_LEAN_COUNTER: f32 = 158.0;
/// Inventory row is closer to the camera (world_y ≈ -0.34h); relic-to-camera
/// vector ≈ [0, -0.74, 0.67] → `arccos(-0.74) ≈ 138°` for direct face-on.
const SHOP_RELIC_LEAN_INVENTORY: f32 = 138.0;

impl ShopScene {
    /// Listed yen cost for the next restock this visit (0 while free restocks remain).
    pub(crate) fn restock_cost(&self, season: crate::core::season::Season) -> u32 {
        if self.remaining_free_restocks > 0 {
            0
        } else {
            season.shop_restock_cost(self.paid_restocks_this_visit)
        }
    }

    /// Normalized hold progress for rumble / HUD ring (0..=1). Stays at 0 while invalid.
    #[inline]
    pub(crate) fn sell_hold_progress(
        &self,
        now: std::time::Instant,
        shop: &crate::game::engine::ShopReadModel,
    ) -> Option<f32> {
        let started = self.west_sell_hold_started?;
        if !self.sell_hold_valid_for(shop) {
            return Some(0.0);
        }
        Some(
            (now.saturating_duration_since(started).as_secs_f32()
                / crate::ui::prompt_hold_ring::hold_act_seconds())
            .clamp(0.0, 1.0),
        )
    }

    pub(crate) fn sell_hold_valid_for(&self, shop: &crate::game::engine::ShopReadModel) -> bool {
        shared::focused_sell_action(
            self.focus,
            self.items.len(),
            &self.zodiac_items,
            &self.talisman_items,
            shop,
        )
        .is_some()
    }

    #[cfg(feature = "game")]
    #[inline]
    pub(crate) fn sell_hold_in_progress(&self) -> bool {
        self.west_sell_hold_started.is_some()
    }

    /// Normalized hold progress for rumble / HUD ring (0..=1). Stays at 0 while invalid.
    pub(crate) fn buy_hold_progress(
        &self,
        now: std::time::Instant,
        run: &crate::game::run::RunState,
        shop: &crate::game::engine::ShopReadModel,
    ) -> Option<f32> {
        let started = self.confirm_buy_hold_started?;
        if !self.buy_hold_valid_for(run, shop) {
            return Some(0.0);
        }
        Some(
            (now.saturating_duration_since(started).as_secs_f32()
                / crate::ui::prompt_hold_ring::hold_act_seconds())
            .clamp(0.0, 1.0),
        )
    }

    #[cfg(feature = "game")]
    #[inline]
    pub(crate) fn buy_hold_in_progress(&self) -> bool {
        self.confirm_buy_hold_started.is_some()
    }

    pub(crate) fn buy_hold_valid_for(
        &self,
        run: &crate::game::run::RunState,
        shop: &crate::game::engine::ShopReadModel,
    ) -> bool {
        shared::focused_buy_action(
            self.focus,
            &self.items,
            &self.zodiac_items,
            &self.talisman_items,
            &self.pack_items,
            run,
            shop,
        )
        .is_some()
    }

    pub(crate) fn trigger_hold_tooltip(
        &mut self,
        run: &crate::game::run::RunState,
        now: Instant,
        focus: Option<ShopFocus>,
    ) {
        let Some(focus) = focus else {
            return;
        };
        if !run.onboarding_hold_tooltip_enabled() {
            return;
        }
        self.hold_tooltip = Some(HoldTooltipState {
            focus,
            until: now + HOLD_TOOLTIP_DURATION,
        });
    }

    pub(crate) fn hold_tooltip_focus(
        &self,
        run: &crate::game::run::RunState,
        now: Instant,
    ) -> Option<ShopFocus> {
        if !run.onboarding_hold_tooltip_enabled() {
            return None;
        }
        self.hold_tooltip
            .filter(|tip| now <= tip.until)
            .map(|tip| tip.focus)
    }

    pub(crate) fn prune_hold_tooltip(&mut self, now: Instant) {
        if self.hold_tooltip.is_some_and(|tip| now > tip.until) {
            self.hold_tooltip = None;
        }
    }

    /// Freeze hold timers while the targeted action cannot succeed.
    pub(crate) fn tick_hold_anchors(
        &mut self,
        now: std::time::Instant,
        _run: &crate::game::run::RunState,
        shop: &crate::game::engine::ShopReadModel,
    ) {
        if let Some(start) = self.west_sell_hold_started {
            let valid = self.sell_hold_valid_for(shop);
            self.west_sell_hold_started = Some(crate::ui::prompt_hold_ring::freeze_hold_anchor(
                start, now, valid,
            ));
        }
        if let Some(start) = self.confirm_buy_hold_started {
            let valid = self.buy_hold_valid_for(_run, shop);
            self.confirm_buy_hold_started = Some(crate::ui::prompt_hold_ring::freeze_hold_anchor(
                start, now, valid,
            ));
        }
    }

    /// Cancel an in-progress sell hold and stop its windup SFX.
    pub(crate) fn cancel_west_sell_hold(&mut self, bus: &mut crate::game::event_bus::EventBus) {
        if self.west_sell_hold_started.is_some() {
            self.west_sell_hold_started = None;
            crate::ui::prompt_hold_ring::end_hold(bus);
        }
    }

    /// Cancel an in-progress buy hold and stop its windup SFX.
    pub(crate) fn cancel_confirm_buy_hold(&mut self, bus: &mut crate::game::event_bus::EventBus) {
        if self.confirm_buy_hold_started.is_some() {
            self.confirm_buy_hold_started = None;
            crate::ui::prompt_hold_ring::end_hold(bus);
        }
    }

    /// Abandon all hold prompts (focus change, cancel, scene exit).
    pub(crate) fn cancel_all_hold_prompts(&mut self, bus: &mut crate::game::event_bus::EventBus) {
        self.cancel_west_sell_hold(bus);
        self.cancel_confirm_buy_hold(bus);
    }

    /// Set focus from a stable slug — used by the screenshot CLI's
    /// `--shop-focus` flag so headless captures can preview hover-only
    /// chrome (focus rings, plaques, spotlights).
    ///
    /// Slugs: `journal`, `leave`, `restock`, `relic:N`,
    /// `ribbon:N`, `talisman:N`, `pack:N`. Returns an error string for
    /// unknown slugs or out-of-range indices, so the CLI can bail
    /// rather than rendering with wrong focus.
    pub fn set_focus_for_screenshot(&mut self, slug: &str) -> Result<(), String> {
        let parse_idx = |kind: &str, rest: &str| -> Result<usize, String> {
            rest.parse::<usize>().map_err(|_| {
                format!("--shop-focus '{kind}:{rest}' — index must be a non-negative integer")
            })
        };
        let focus = if let Some(rest) = slug.strip_prefix("relic:") {
            ShopFocus::Relic(parse_idx("relic", rest)?)
        } else if let Some(rest) = slug.strip_prefix("ribbon:") {
            ShopFocus::Ribbon(parse_idx("ribbon", rest)?)
        } else if let Some(rest) = slug.strip_prefix("talisman:") {
            ShopFocus::Talisman(parse_idx("talisman", rest)?)
        } else if let Some(rest) = slug.strip_prefix("pack:") {
            ShopFocus::Pack(PICK_TILE_PACK_BASE + parse_idx("pack", rest)? as u32)
        } else {
            match slug {
                "journal" => ShopFocus::Dish(PICK_JOURNAL_BOOK),
                "leave" => ShopFocus::NextRound,
                "restock" => ShopFocus::Restock,
                other => {
                    return Err(format!(
                        "--shop-focus '{other}' — supported: journal, leave, restock, relic:N, ribbon:N, talisman:N, pack:N"
                    ));
                }
            }
        };
        self.focus = Some(focus);
        Ok(())
    }

    /// Headless screenshot: initial orbit for [`crate::scenes::ItemInspectScene`]
    /// from current focus and stock. Returns `None` when focus is missing,
    /// not inspectable, or orbit math fails.
    pub fn item_inspect_orbit_for_screenshot(
        &self,
        w: f32,
        h: f32,
        run: &crate::game::run::RunState,
    ) -> Option<crate::scenes::object3d_inspect::ItemInspectOrbitState> {
        let focus = self.focus?;
        if !shop_focus_inspectable(focus) {
            return None;
        }
        let env_h = self.drawn_room_gltf_height_scale.get();
        let shop = GameEngine::read_shop(run);
        view::shop_item_inspect_orbit_for_focus(self, w, h, env_h, &shop, focus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::game_mode::GameMode;

    #[test]
    fn shop_focus_screenshot_slugs_reject_legacy_aliases() {
        let mut run = crate::game::run::RunState::new(GameMode::standard());
        let mut shop = ShopScene::new(&mut run, &crate::core::progression::PlayerProgress::new());

        for slug in ["bell", "next-round", "abacus"] {
            assert!(
                shop.set_focus_for_screenshot(slug).is_err(),
                "legacy alias '{slug}' should be rejected"
            );
        }
        assert!(shop.set_focus_for_screenshot("leave").is_ok());
        assert!(shop.set_focus_for_screenshot("restock").is_ok());
    }

    #[test]
    fn patron_gift_shop_always_contains_a_free_relic() {
        let mut run = crate::game::run::RunState::new(GameMode::standard());
        run.tag_patron_gift = 1;

        let shop = ShopScene::new(&mut run, &crate::core::progression::PlayerProgress::new());

        assert!(!shop.items.is_empty());
        assert!(shop.items.iter().any(|item| item.price == 0));
        assert_eq!(run.tag_patron_gift, 0);
    }

    #[test]
    fn patron_gift_free_relic_blocked_when_inventory_full() {
        use crate::core::relic::{RelicId, all_relic_defs};
        use crate::game::event_bus::EventBus;

        let mut run = crate::game::run::RunState::new(GameMode::standard());
        run.relics.active = vec![
            RelicId::PairPower,
            RelicId::TripletBoost,
            RelicId::SequenceSurge,
            RelicId::HonorFury,
            RelicId::DragonRage,
        ];
        run.recompute_capacities();
        assert!(run.relics.is_full());

        // Patron gift zeroes a random shop relic; BrocadePouch is the one
        // exception that can still be claimed when full. Use a fixed offer so
        // this test does not depend on shop RNG.
        let free_relic = RelicId::AntTrail;
        assert_ne!(free_relic, RelicId::BrocadePouch);
        let def = all_relic_defs()
            .iter()
            .find(|d| d.id == free_relic)
            .expect("AntTrail relic def");
        let mut items = vec![ShopItem {
            relic: def.id,
            name: def.name,
            rarity: def.rarity,
            price: 0,
            sold: false,
        }];

        let mut bus = EventBus::default();
        apply_shop_action(
            ShopAction::BuyCard(0),
            &mut items,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut [],
            &mut run,
            &mut bus,
        );

        assert_eq!(run.relics.active.len(), 5);
        assert!(!run.relics.active.contains(&free_relic));
    }

    #[test]
    fn shop_entry_clears_stale_finished_zodiac_celebration() {
        let mut run = crate::game::run::RunState::new(GameMode::standard());
        run.finished_zodiac_celebration = Some(("Riichi", 2));
        run.pending_shop_focus_snap_after_celebration = true;

        let _shop =
            ShopScene::new(&mut run, &crate::core::progression::PlayerProgress::new());

        assert!(run.finished_zodiac_celebration.is_none());
        assert!(!run.pending_shop_focus_snap_after_celebration);
    }

    #[test]
    fn stacked_patron_gift_shop_zeros_multiple_relics() {
        let mut run = crate::game::run::RunState::new(GameMode::standard());
        run.tag_patron_gift = 2;

        let shop = ShopScene::new(&mut run, &crate::core::progression::PlayerProgress::new());

        assert!(shop.items.iter().filter(|item| item.price == 0).count() >= 2);
        assert_eq!(run.tag_patron_gift, 0);
    }

    #[test]
    fn rich_stock_shop_starts_with_two_extra_relics() {
        let mut run = crate::game::run::RunState::new(GameMode::standard());
        run.tag_rich_stock = 1;

        let shop = ShopScene::new(&mut run, &crate::core::progression::PlayerProgress::new());

        assert!(shop.items.len() >= 2);
        assert_eq!(run.tag_rich_stock, 0);
    }

    #[test]
    fn stacked_rich_stock_shop_adds_four_extra_relics() {
        let mut run = crate::game::run::RunState::new(GameMode::standard());
        run.tag_rich_stock = 2;

        let shop = ShopScene::new(&mut run, &crate::core::progression::PlayerProgress::new());

        assert!(shop.items.len() >= 4);
        assert_eq!(run.tag_rich_stock, 0);
    }

    #[test]
    fn shop_only_rolls_unlocked_relics() {
        let relics = RelicState::default();
        let available_relics = vec![RelicId::PairPower];
        let mode = GameMode::standard();
        let run = crate::game::run::RunState::new(mode.clone());

        let (items, _, _, _) = actions::generate_shop_stock(
            &relics,
            &available_relics,
            1,
            crate::game::run::RelicShopPoolExtinction::default(),
            &mode,
            &run,
        );

        assert!(!items.is_empty());
        assert!(items.iter().all(|item| item.relic == RelicId::PairPower));
    }

    #[test]
    fn shop_zodiac_stock_requires_scored_yaku() {
        use crate::core::yaku::YakuKind;
        let relics = RelicState::default();
        let available = vec![RelicId::PairPower];
        let mode = GameMode::standard();
        let ex = crate::game::run::RelicShopPoolExtinction::default();
        let fresh_run = crate::game::run::RunState::new(mode.clone());
        for _ in 0..48 {
            let (_, zodiacs, _, _) =
                actions::generate_shop_stock(&relics, &available, 0, ex, &mode, &fresh_run);
            assert!(zodiacs.is_empty(), "fresh profile should offer no zodiacs");
        }

        let mut unlocked_run = crate::game::run::RunState::new(mode.clone());
        unlocked_run
            .profile_yaku_scored
            .insert(YakuKind::KokushiMusou);
        let mut saw_qilin = false;
        for _ in 0..64 {
            let (_, zodiacs, _, _) =
                actions::generate_shop_stock(&relics, &available, 0, ex, &mode, &unlocked_run);
            if zodiacs
                .iter()
                .any(|item| matches!(item.consumable, Consumable::Zodiac(ZodiacKind::Qilin)))
            {
                saw_qilin = true;
                break;
            }
        }
        assert!(saw_qilin);
    }
}
