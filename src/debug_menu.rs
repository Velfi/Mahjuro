//! Native OS menubar "Debug" menu using muda.
//!
//! Provides debug shortcuts for setting player level, gold, relics, and card inventory.

use muda::{Menu, MenuEvent, MenuId, MenuItem, Submenu};

use crate::core::boss::{ALL_BOSSES, BossKind, FINAL_BOSSES};
use crate::core::relic::{RelicId, all_relic_defs};
use crate::core::talisman::TalismanKind;
use crate::core::zodiac::ZodiacKind;

/// Identifies which debug action was triggered.
#[derive(Clone, Debug)]
pub enum DebugAction {
    SetLevel(u32),
    SetGold(u32),
    AddRelic(RelicId),
    ClearRelics,
    AddTalisman(TalismanKind),
    AddZodiac(ZodiacKind),
    ClearConsumables,
    /// Replace the current ante's `upcoming_boss` and re-resolve its effect
    /// (so reactive variants like Mirror/Tax Collector pick fresh).
    SetBoss(BossKind),
    ToggleShowFps,
    /// Open the in-game debug visibility modal — a checkbox panel for
    /// hiding individual gameplay HUD elements (tiles, candles, the two
    /// hanging plaques, the inventory dish) so the procedural 3D scene
    /// underneath can be inspected.
    OpenDebugVisibility,
    /// Toggle a debug overlay that draws three colored bars (red = +X,
    /// green = +Y, blue = +Z) anchored at the camera's look target. Useful
    /// for disambiguating world-space directions when iterating on
    /// placements.
    ToggleWorldAxes,
    OpenTuning,
    OpenSfxTest,
    BlowWindGust,
    /// Capture GPU pass timings averaged over the next 100 rendered frames
    /// and log the result. Only meaningful on backends that support
    /// `wgpu::Features::TIMESTAMP_QUERY`.
    ProfileGpu,
    /// Arm a one-shot picker: the next mouse click in the game world is
    /// hit-tested against every known scene object and the matched object's
    /// name is logged. Activating this while already armed disarms it.
    ArmObjectHitTest,
}

/// Holds the menu bar and maps MenuIds to DebugActions.
pub struct DebugMenuBar {
    #[allow(dead_code)]
    menu: Menu,
    mappings: Vec<(MenuId, DebugAction)>,
}

impl DebugMenuBar {
    /// Build and install the debug menu bar. Must be called on the main thread.
    pub fn new() -> Self {
        let menu = Menu::new();

        // --- App submenu (macOS convention: first submenu is the app menu) ---
        let app_menu = Submenu::new("Mahjuro", true);
        let _ = menu.append(&app_menu);

        // --- Debug submenu ---
        let debug_menu = Submenu::new("Debug", true);

        let mut mappings = Vec::new();

        // Show FPS toggle.
        let fps_item = MenuItem::new("Show FPS", true, None);
        mappings.push((fps_item.id().clone(), DebugAction::ToggleShowFps));
        let _ = debug_menu.append(&fps_item);

        // In-game checkbox modal that can hide tiles, candles, the blind
        // plaque, the scoring placard, and the inventory dish independently.
        // Lets us inspect the procedural 3D scene without the HUD in the way.
        let visibility_item = MenuItem::new("Visibility...", true, None);
        mappings.push((
            visibility_item.id().clone(),
            DebugAction::OpenDebugVisibility,
        ));
        let _ = debug_menu.append(&visibility_item);

        // World-axes overlay toggle (red = +X, green = +Y, blue = +Z).
        let axes_item = MenuItem::new("World Axes Overlay", true, None);
        mappings.push((axes_item.id().clone(), DebugAction::ToggleWorldAxes));
        let _ = debug_menu.append(&axes_item);

        // Cascade tuning overlay.
        let tuning_item = MenuItem::new("Cascade Tuning...", true, None);
        mappings.push((tuning_item.id().clone(), DebugAction::OpenTuning));
        let _ = debug_menu.append(&tuning_item);

        // Sound effects test overlay.
        let sfx_item = MenuItem::new("Sound Effects Test...", true, None);
        mappings.push((sfx_item.id().clone(), DebugAction::OpenSfxTest));
        let _ = debug_menu.append(&sfx_item);

        // Spawn a strong wind gust at the candle row so the flame's
        // wind reaction is observable on demand. Mirrors pressing `B`
        // in the gameplay scene.
        let wind_item = MenuItem::new("Blow Wind Gust", true, None);
        mappings.push((wind_item.id().clone(), DebugAction::BlowWindGust));
        let _ = debug_menu.append(&wind_item);

        // Capture GPU pass timings over the next 100 frames. Result is
        // logged via `log::info!`.
        let profile_item = MenuItem::new("Profile GPU (100 frames)", true, None);
        mappings.push((profile_item.id().clone(), DebugAction::ProfileGpu));
        let _ = debug_menu.append(&profile_item);

        // Arm a one-shot debug picker — the next click is hit-tested
        // against every known scene object and its name logged.
        let hit_test_item = MenuItem::new("Object Hit Test", true, None);
        mappings.push((hit_test_item.id().clone(), DebugAction::ArmObjectHitTest));
        let _ = debug_menu.append(&hit_test_item);

        // Set Level submenu (levels 1-7).
        let level_sub = Submenu::new("Set Player Level", true);
        for lvl in 1..=7u32 {
            let item = MenuItem::new(format!("Level {lvl}"), true, None);
            mappings.push((item.id().clone(), DebugAction::SetLevel(lvl)));
            let _ = level_sub.append(&item);
        }
        let _ = debug_menu.append(&level_sub);

        // Set Gold submenu.
        let gold_sub = Submenu::new("Set Gold", true);
        for &amount in &[0u32, 10, 50, 100, 500, 9999] {
            let item = MenuItem::new(format!("{amount} Gold"), true, None);
            mappings.push((item.id().clone(), DebugAction::SetGold(amount)));
            let _ = gold_sub.append(&item);
        }
        let _ = debug_menu.append(&gold_sub);

        // Relic Inventory submenu.
        let relic_sub = Submenu::new("Relic Inventory", true);
        let clear_item = MenuItem::new("Clear All Relics", true, None);
        mappings.push((clear_item.id().clone(), DebugAction::ClearRelics));
        let _ = relic_sub.append(&clear_item);

        let add_sub = Submenu::new("Add Relic", true);
        for def in all_relic_defs() {
            let item = MenuItem::new(def.name, true, None);
            mappings.push((item.id().clone(), DebugAction::AddRelic(def.id)));
            let _ = add_sub.append(&item);
        }
        let _ = relic_sub.append(&add_sub);
        let _ = debug_menu.append(&relic_sub);

        // Consumable Inventory submenu — talismans + zodiacs share slots, so
        // expose both under one parent. Capacity is auto-expanded when full,
        // mirroring the relic add behavior above.
        let consumable_sub = Submenu::new("Consumable Inventory", true);
        let clear_cons_item = MenuItem::new("Clear All Consumables", true, None);
        mappings.push((clear_cons_item.id().clone(), DebugAction::ClearConsumables));
        let _ = consumable_sub.append(&clear_cons_item);

        let add_talisman_sub = Submenu::new("Add Talisman", true);
        for &kind in TalismanKind::all() {
            let item = MenuItem::new(kind.name(), true, None);
            mappings.push((item.id().clone(), DebugAction::AddTalisman(kind)));
            let _ = add_talisman_sub.append(&item);
        }
        let _ = consumable_sub.append(&add_talisman_sub);

        let add_zodiac_sub = Submenu::new("Add Zodiac", true);
        for &kind in ZodiacKind::all() {
            let item = MenuItem::new(kind.name(), true, None);
            mappings.push((item.id().clone(), DebugAction::AddZodiac(kind)));
            let _ = add_zodiac_sub.append(&item);
        }
        let _ = consumable_sub.append(&add_zodiac_sub);
        let _ = debug_menu.append(&consumable_sub);

        // Boss override submenu — pick any boss (regular or final) and the
        // current ante's upcoming_boss is replaced + re-resolved. Useful for
        // testing reactive bosses (Mirror, Tax Collector) against specific
        // run states without rerolling antes until the right boss appears.
        let boss_sub = Submenu::new("Set Current Boss", true);
        for def in ALL_BOSSES.iter().chain(FINAL_BOSSES.iter()) {
            let item = MenuItem::new(format!("{} [{}]", def.name, def.tier.label()), true, None);
            mappings.push((item.id().clone(), DebugAction::SetBoss(def.kind)));
            let _ = boss_sub.append(&item);
        }
        let _ = debug_menu.append(&boss_sub);

        let _ = menu.append(&debug_menu);

        // Install as macOS global app menu.
        #[cfg(target_os = "macos")]
        menu.init_for_nsapp();

        Self { menu, mappings }
    }

    /// Poll for pending debug actions (non-blocking). Call once per frame.
    pub fn poll(&self) -> Vec<DebugAction> {
        let mut actions = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            for (id, action) in &self.mappings {
                if event.id() == id {
                    actions.push(action.clone());
                    break;
                }
            }
        }
        actions
    }
}
