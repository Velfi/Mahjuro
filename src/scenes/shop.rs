//! Shop scene — between rounds; player can buy relics with gold (`THEME.md` storeroom + `shop.glb`).
//!
//! **[`ShopScene`]** is the [`crate::scenes::Scene`] variant; rendering and hit layout live in the internal `view` module.

mod actions;
mod draw;
mod layout;
pub(crate) mod pack_celebration;
mod restock_exit;
pub(crate) mod pick_ids;
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
    N_TILE_PACKS, PICK_COIN_DISH, PICK_JOURNAL_BOOK, PICK_LEAVE_PROP, PICK_REROLL_PROP,
    PICK_TILE_PACK_BASE,
};

use rand::seq::SliceRandom;

use std::time::Instant;

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
use crate::ui::focus_nav::{FocusDir, focus_target_at_cursor, pick_neighbor};
use crate::ui::input::{InputMode, UiAction};

use super::pause_menu::PauseMenu;
use super::hallway::HallwayScene;
pub(crate) use super::{Scene, SceneTransition, UpdateCtx};

pub struct ShopScene {
    mode: ShopMode,
    items: Vec<ShopItem>,
    zodiac_items: Vec<ConsumableShopItem>,
    talisman_items: Vec<ConsumableShopItem>,
    /// Tile packs for sale this shop visit (always up to `N_TILE_PACKS`).
    pack_items: Vec<TilePackShopItem>,
    /// Current reroll cost — starts at `REROLL_BASE_COST` and increases by
    /// `REROLL_COST_INCREMENT` each time the player rerolls this shop visit.
    reroll_cost: u32,
    /// Skip-tag free rerolls still owed this visit (consumed one at a time).
    remaining_free_rerolls: u32,
    pause_menu: PauseMenu,
    /// Currently focused shop element. Starts on the first for-sale shelf item
    /// (or the leave bell when the shelf is empty). In cursor mode, hover follows
    /// the 3D pick when present; otherwise screen rects (shelf slots + HUD buttons).
    focus: Option<ShopFocus>,
    /// Focus rect graph captured at the end of the previous `draw_frame`,
    /// consumed by `update()` for cursor hit-tests and spatial navigation.
    /// One frame stale — same pattern as `projected_relic_rects` and the
    /// gameplay scene's identical mechanism. Wrapped in a `RefCell` because
    /// `draw_frame` takes `&self` but needs to update this stash.
    last_focus_rects: std::cell::RefCell<Vec<(ShopFocus, [f32; 4])>>,
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
    /// West-face hold-to-sell (gamepad West / **Q**): press time when a hold is in progress.
    west_sell_hold_started: Option<std::time::Instant>,
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

/// Click id for the `?` glossary badge in the shop HUD.
const SHOP_HELP_BADGE_ID: u32 = 0x9100;

/// Click id for the catch-all 3D-hit dispatcher. When clicked, the shop's
/// update() routes the click based on `UpdateCtx::picked_shop_object`.
pub const SHOP_3D_HIT_ID: u32 = 0x9200;
/// Hold-to-sell duration (gamepad West / keyboard **Q**). Drives HUD ring + sell gate.
pub(crate) const SHOP_SELL_HOLD_SECONDS: f32 = 1.0;
/// Click id for the Leave / advance 2D button (kept for focus-nav compat).
const SHOP_NEXT_ROUND_ID: u32 = 0x9300;
/// Click id for the Reroll 2D button (kept for focus-nav compat).
const SHOP_REROLL_ID: u32 = 0x9400;
/// Floating sell button for hovered owned relics.
const SHOP_SELL_RELIC_BASE: u32 = 0x9500;
/// Floating sell button for hovered owned consumables.
const SHOP_SELL_CONSUMABLE_BASE: u32 = 0x9600;
/// How long a relic glow + wiggle lasts after activation.
const RELIC_GLOW_LIFETIME: std::time::Duration = std::time::Duration::from_millis(900);
/// How much the reroll cost increases per use within a single shop visit.
const REROLL_COST_INCREMENT: u32 = 5;

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
    #[inline]
    #[cfg(feature = "game")]
    pub(crate) fn sell_hold_in_progress(&self) -> bool {
        self.west_sell_hold_started.is_some()
    }

    /// Normalized hold progress for rumble / HUD ring (0..=1).
    #[inline]
    #[cfg(feature = "game")]
    pub(crate) fn sell_hold_progress(&self, now: std::time::Instant) -> Option<f32> {
        self.west_sell_hold_started.map(|started| {
            (now.saturating_duration_since(started).as_secs_f32() / SHOP_SELL_HOLD_SECONDS)
                .clamp(0.0, 1.0)
        })
    }

    /// Set focus from a stable slug — used by the screenshot CLI's
    /// `--shop-focus` flag so headless captures can preview hover-only
    /// chrome (focus rings, plaques, spotlights).
    ///
    /// Slugs: `journal`, `bell`, `abacus`, `relic:N`,
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
                "bell" | "leave" | "next-round" => ShopFocus::NextRound,
                "abacus" | "reroll" => ShopFocus::Reroll,
                other => {
                    return Err(format!(
                        "--shop-focus '{other}' — supported: journal, bell, abacus, relic:N, ribbon:N, talisman:N, pack:N"
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
    use crate::persistence::TileMaterial;

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
    fn season_scales_shop_prices() {
        use crate::core::season::Season;
        let relics = RelicState::default();
        let available = vec![RelicId::PairPower];
        let spring = GameMode::with_material_and_season(TileMaterial::Bamboo, Season::Spring);
        let winter = GameMode::with_material_and_season(TileMaterial::Bamboo, Season::Winter);
        let spring_run = crate::game::run::RunState::new(spring.clone());
        let winter_run = crate::game::run::RunState::new(winter.clone());

        let (spring_items, _, _, _) = actions::generate_shop_stock(
            &relics,
            &available,
            1,
            crate::game::run::RelicShopPoolExtinction::default(),
            &spring,
            &spring_run,
        );
        let (winter_items, _, _, _) = actions::generate_shop_stock(
            &relics,
            &available,
            1,
            crate::game::run::RelicShopPoolExtinction::default(),
            &winter,
            &winter_run,
        );

        let spring_price = spring_items[0].price;
        let winter_price = winter_items[0].price;
        assert!(
            winter_price > spring_price,
            "Winter {} should exceed Spring {}",
            winter_price,
            spring_price,
        );
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
            if zodiacs.iter().any(|item| {
                matches!(item.consumable, Consumable::Zodiac(ZodiacKind::Qilin))
            }) {
                saw_qilin = true;
                break;
            }
        }
        assert!(saw_qilin);
    }
}
