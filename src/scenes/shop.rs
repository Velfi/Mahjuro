//! Shop scene — between rounds; player can buy relics with gold.
//!
//! Renders as a night **mountain path kiosk**: a wide counter with four
//! back-row stalls (relics, tile pack, talismans, ribbons), owned inventory
//! in three bottom trays, and top chrome (path sign, lamp, leave). Hovering
//! an object turns on a point-light spotlight; tooltips and a central
//! selected-item panel show name + Buy/Sell CTA.

mod actions;
mod draw;
mod shared;
mod layout;
mod update;

use self::layout::*;
use self::shared::*;

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
    GameEngine, ShopCommand, ShopCommandData, ShopReadModel, consumable_sell_price_for_mode,
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

/// Pitch relic cuboids toward the camera (`rot_rx_rz_deg`).
/// The relic front cap is at local +Y; pitching past 90° tilts it to face -Y
/// (toward the camera). Camera is at (0, -0.72h, 0.34h); counter relics sit at
/// world_y ≈ +0.19h, so the relic-to-camera vector is roughly [0, -0.95, 0.31].
/// `arccos(-0.95) ≈ 162°` makes the face point directly at the camera.
const SHOP_RELIC_LEAN_COUNTER: f32 = 158.0;
/// Inventory row is closer to the camera (world_y ≈ -0.34h); relic-to-camera
/// vector ≈ [0, -0.74, 0.67] → `arccos(-0.74) ≈ 138°` for direct face-on.
const SHOP_RELIC_LEAN_INVENTORY: f32 = 138.0;

impl SceneBehavior for ShopScene {
    fn pause_options_overlay(&self) -> Option<&super::options::OptionsScene> {
        self.pause_options_overlay_impl()
    }

    fn has_blocking_overlay(&self) -> bool {
        self.has_blocking_overlay_impl()
    }

    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.update_impl(ctx)
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        self.draw_frame_impl(ctx)
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
        let mode = GameMode::standard();

        let (items, _, _, _) = generate_shop_stock(&relics, &available_relics, 1, false, &mode);

        assert!(!items.is_empty());
        assert!(items.iter().all(|item| item.relic == RelicId::PairPower));
    }

    #[test]
    fn stake_scales_shop_prices() {
        use crate::core::stake::Stake;
        let relics = RelicState::default();
        let available = vec![RelicId::PairPower];
        let spring = GameMode::with_material_and_stake(TileMaterial::Bamboo, Stake::Spring);
        let winter = GameMode::with_material_and_stake(TileMaterial::Bamboo, Stake::Winter);

        let (spring_items, _, _, _) = generate_shop_stock(&relics, &available, 1, false, &spring);
        let (winter_items, _, _, _) = generate_shop_stock(&relics, &available, 1, false, &winter);

        let spring_price = spring_items[0].price;
        let winter_price = winter_items[0].price;
        assert!(
            winter_price > spring_price,
            "Winter {} should exceed Spring {}",
            winter_price,
            spring_price,
        );
    }
}
