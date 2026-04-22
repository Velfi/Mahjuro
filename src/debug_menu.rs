//! Native OS menubar "Debug" menu using muda.
//!
//! Provides debug shortcuts for setting player level, gold, relics, and card inventory.
//!
//! ## Cross-platform installation
//!
//! - **macOS**: installed as the global NSApp menu via `init_for_nsapp()`.
//! - **Windows**: installed onto the main window's HWND via `init_for_hwnd()`.
//!   The HWND is extracted from the winit window's `RawWindowHandle::Win32`.
//! - **Linux**: muda requires GTK on Linux, which the rest of the app does not
//!   pull in. The menu is built (so `poll()` continues to work for any
//!   programmatically-injected events) but is *not* attached to the window —
//!   Linux users reach the same actions via in-app keyboard shortcuts instead.

use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use winit::window::Window;

use crate::core::boss::{ALL_BOSSES, BossKind, FINAL_BOSSES};
use crate::core::relic::{RelicId, all_relic_defs};
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
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
    /// Replace the current dora indicator with a single tile of the given
    /// face. Clobbers any extra dora revealed by Dora Crown.
    SetDora(Suit, u8),
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
    OpenCameraDebug,
    /// Open the in-game shop-smoke tuning overlay — live sliders for the
    /// shop back-wall smoke curtain's density, radius, velocity, and lift.
    OpenSmokeDebug,
    /// Open the volumetric tuning overlay — global dust strength (and any
    /// future volumetric knobs).
    OpenVolumetricDebug,
    BlowWindGust,
    /// Capture GPU pass timings averaged over the next 100 rendered frames
    /// and log the result. Only meaningful on backends that support
    /// `wgpu::Features::TIMESTAMP_QUERY`.
    ProfileGpu,
    /// Arm a one-shot picker: the next mouse click in the game world is
    /// hit-tested against every known scene object and the matched object's
    /// name is logged. Activating this while already armed disarms it.
    ArmObjectHitTest,
    /// Reroll the shop stock for free (no gold cost, no cost increment).
    RerollShop,
    /// Force-open a random tile pack celebration (free, ignores shop stock).
    OpenPack,
    /// Spawn a burst of demo score popups streaming toward the reel —
    /// visually exercises the chips/mult/gold polychrome streaming effect
    /// without needing to play a hand. Only meaningful in the gameplay scene.
    DemoCascade,
    /// Spawn a blank test modal overlay to verify overlay blocking behavior.
    TestOverlay,
    /// Jump directly to the victory scene for presentation/debugging.
    ShowVictoryScreen,
    /// Jump directly to the defeat scene for presentation/debugging.
    ShowDefeatScreen,
    /// Enter (or exit) arrange mode: click an object, then use arrow keys to
    /// nudge its position (X/Y) or Shift+arrow to rotate it (Z/X). Press
    /// Enter to confirm and copy the result to the clipboard.
    ToggleArrangeMode,
    /// Push the material viewer pushdown scene onto the overlay stack.
    /// Shows one preview orb per `MaterialKind` for visual inspection.
    OpenMaterialViewer,
    /// Push the transition playground scene onto the overlay stack.
    OpenTransitionPlayground,
    /// Open a simple in-app About modal. Used on macOS to avoid the native
    /// About panel's icon conversion path in `muda`.
    OpenAbout,
}

/// Holds the menu bar and maps MenuIds to DebugActions.
pub struct DebugMenuBar {
    // Must be retained for the lifetime of the installed native menubar.
    // On macOS, AppKit can invoke menu item actions long after
    // `init_for_nsapp()`, so dropping the Rust-side menu object leaves the
    // native callbacks dangling.
    _retained_menu: Menu,
    mappings: Vec<(MenuId, DebugAction)>,
}

impl DebugMenuBar {
    /// Build and install the debug menu bar. Must be called on the main thread,
    /// after the window is created (Windows needs the HWND).
    pub fn new(window: &Window) -> Self {
        let menu = Menu::new();
        let mut mappings = Vec::new();

        // --- App submenu (macOS convention: first submenu is the app menu) ---
        let app_menu = Submenu::new("Mahjuro", true);
        #[cfg(target_os = "macos")]
        {
            let about = MenuItem::new("About Mahjuro", true, None);
            mappings.push((about.id().clone(), DebugAction::OpenAbout));
            let _ = app_menu.append(&about);
            let _ = app_menu.append(&PredefinedMenuItem::separator());
            let _ = app_menu.append(&PredefinedMenuItem::hide(None));
            let _ = app_menu.append(&PredefinedMenuItem::hide_others(None));
            let _ = app_menu.append(&PredefinedMenuItem::show_all(None));
            let _ = app_menu.append(&PredefinedMenuItem::separator());
            let _ = app_menu.append(&PredefinedMenuItem::quit(None));
        }
        let _ = menu.append(&app_menu);

        // --- Debug submenu ---
        let debug_menu = Submenu::new("Debug", true);

        // ── Overlays submenu ─────────────────────────────────────────────────
        // Toggles and modal panels that live on top of the gameplay view.
        let overlays_sub = Submenu::new("Overlays", true);

        let fps_item = MenuItem::new("Show FPS", true, None);
        mappings.push((fps_item.id().clone(), DebugAction::ToggleShowFps));
        let _ = overlays_sub.append(&fps_item);

        let visibility_item = MenuItem::new("Visibility...", true, None);
        mappings.push((
            visibility_item.id().clone(),
            DebugAction::OpenDebugVisibility,
        ));
        let _ = overlays_sub.append(&visibility_item);

        let axes_item = MenuItem::new("World Axes Overlay", true, None);
        mappings.push((axes_item.id().clone(), DebugAction::ToggleWorldAxes));
        let _ = overlays_sub.append(&axes_item);

        let _ = debug_menu.append(&overlays_sub);

        // ── Tuning submenu ───────────────────────────────────────────────────
        // Live-editable parameter panels for FX / camera / materials.
        let tuning_sub = Submenu::new("Tuning", true);

        let tuning_item = MenuItem::new("Cascade...", true, None);
        mappings.push((tuning_item.id().clone(), DebugAction::OpenTuning));
        let _ = tuning_sub.append(&tuning_item);

        let camera_item = MenuItem::new("Camera...", true, None);
        mappings.push((camera_item.id().clone(), DebugAction::OpenCameraDebug));
        let _ = tuning_sub.append(&camera_item);

        let smoke_item = MenuItem::new("Shop Smoke...", true, None);
        mappings.push((smoke_item.id().clone(), DebugAction::OpenSmokeDebug));
        let _ = tuning_sub.append(&smoke_item);

        let vol_item = MenuItem::new("Volumetric...", true, None);
        mappings.push((vol_item.id().clone(), DebugAction::OpenVolumetricDebug));
        let _ = tuning_sub.append(&vol_item);

        let sfx_item = MenuItem::new("Sound Effects Test...", true, None);
        mappings.push((sfx_item.id().clone(), DebugAction::OpenSfxTest));
        let _ = tuning_sub.append(&sfx_item);

        let _ = debug_menu.append(&tuning_sub);

        // ── Tools submenu ────────────────────────────────────────────────────
        // One-shot dev tools: profiling, picking, layout capture.
        let tools_sub = Submenu::new("Tools", true);

        let profile_item = MenuItem::new("Profile GPU (100 frames)", true, None);
        mappings.push((profile_item.id().clone(), DebugAction::ProfileGpu));
        let _ = tools_sub.append(&profile_item);

        let hit_test_item = MenuItem::new("Object Hit Test", true, None);
        mappings.push((hit_test_item.id().clone(), DebugAction::ArmObjectHitTest));
        let _ = tools_sub.append(&hit_test_item);

        let arrange_item = MenuItem::new("Arrange Mode", true, None);
        mappings.push((arrange_item.id().clone(), DebugAction::ToggleArrangeMode));
        let _ = tools_sub.append(&arrange_item);

        let _ = debug_menu.append(&tools_sub);

        // ── Scene Jumps submenu ──────────────────────────────────────────────
        // Jump to scenes or push test overlays without playing through.
        let jumps_sub = Submenu::new("Scene Jumps", true);

        let victory_item = MenuItem::new("Show Victory Screen", true, None);
        mappings.push((victory_item.id().clone(), DebugAction::ShowVictoryScreen));
        let _ = jumps_sub.append(&victory_item);

        let defeat_item = MenuItem::new("Show Defeat Screen", true, None);
        mappings.push((defeat_item.id().clone(), DebugAction::ShowDefeatScreen));
        let _ = jumps_sub.append(&defeat_item);

        let material_viewer_item = MenuItem::new("Material Viewer...", true, None);
        mappings.push((
            material_viewer_item.id().clone(),
            DebugAction::OpenMaterialViewer,
        ));
        let _ = jumps_sub.append(&material_viewer_item);

        let transition_playground_item = MenuItem::new("Transition Playground...", true, None);
        mappings.push((
            transition_playground_item.id().clone(),
            DebugAction::OpenTransitionPlayground,
        ));
        let _ = jumps_sub.append(&transition_playground_item);

        let test_overlay_item = MenuItem::new("Test Overlay", true, None);
        mappings.push((test_overlay_item.id().clone(), DebugAction::TestOverlay));
        let _ = jumps_sub.append(&test_overlay_item);

        let _ = debug_menu.append(&jumps_sub);

        // ── Cheats submenu ───────────────────────────────────────────────────
        // Gameplay-affecting shortcuts: free items, set resources, force state.
        let cheats_sub = Submenu::new("Cheats", true);

        let reroll_item = MenuItem::new("Reroll Shop", true, None);
        mappings.push((reroll_item.id().clone(), DebugAction::RerollShop));
        let _ = cheats_sub.append(&reroll_item);

        let open_pack_item = MenuItem::new("Open Tile Pack", true, None);
        mappings.push((open_pack_item.id().clone(), DebugAction::OpenPack));
        let _ = cheats_sub.append(&open_pack_item);

        let demo_cascade_item = MenuItem::new("Demo Score Cascade", true, None);
        mappings.push((demo_cascade_item.id().clone(), DebugAction::DemoCascade));
        let _ = cheats_sub.append(&demo_cascade_item);

        let wind_item = MenuItem::new("Blow Wind Gust", true, None);
        mappings.push((wind_item.id().clone(), DebugAction::BlowWindGust));
        let _ = cheats_sub.append(&wind_item);

        let _ = cheats_sub.append(&PredefinedMenuItem::separator());

        // Set Level submenu (levels 1-7).
        let level_sub = Submenu::new("Set Player Level", true);
        for lvl in 1..=7u32 {
            let item = MenuItem::new(format!("Level {lvl}"), true, None);
            mappings.push((item.id().clone(), DebugAction::SetLevel(lvl)));
            let _ = level_sub.append(&item);
        }
        let _ = cheats_sub.append(&level_sub);

        // Set Gold submenu.
        let gold_sub = Submenu::new("Set Gold", true);
        for &amount in &[0u32, 10, 50, 100, 500, 9999] {
            let item = MenuItem::new(format!("{amount} Gold"), true, None);
            mappings.push((item.id().clone(), DebugAction::SetGold(amount)));
            let _ = gold_sub.append(&item);
        }
        let _ = cheats_sub.append(&gold_sub);

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
        let _ = cheats_sub.append(&relic_sub);

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
        let _ = cheats_sub.append(&consumable_sub);

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
        let _ = cheats_sub.append(&boss_sub);

        // Set Dora submenu — pick any non-bonus tile face (number suits,
        // winds, dragons) and the current dora indicator is replaced.
        // Grouped into sub-submenus per suit to keep the list browsable.
        let dora_sub = Submenu::new("Set Dora", true);
        for (suit_label, suit, max_rank) in [
            ("Characters (m)", Suit::Characters, 9u8),
            ("Bamboos (s)", Suit::Bamboos, 9u8),
            ("Circles (p)", Suit::Circles, 9u8),
            ("Winds", Suit::Wind, 4u8),
            ("Dragons", Suit::Dragon, 3u8),
        ] {
            let suit_sub = Submenu::new(suit_label, true);
            for rank in 1..=max_rank {
                let label = Tile::new(suit, rank, 0).full_name();
                let item = MenuItem::new(label, true, None);
                mappings.push((item.id().clone(), DebugAction::SetDora(suit, rank)));
                let _ = suit_sub.append(&item);
            }
            let _ = dora_sub.append(&suit_sub);
        }
        let _ = cheats_sub.append(&dora_sub);

        let _ = debug_menu.append(&cheats_sub);

        let _ = menu.append(&debug_menu);

        install_menu(&menu, window);

        Self {
            _retained_menu: menu,
            mappings,
        }
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

/// Per-OS dispatch for attaching the menu to the active app/window.
#[cfg(target_os = "macos")]
fn install_menu(menu: &Menu, _window: &Window) {
    menu.init_for_nsapp();
}

#[cfg(target_os = "windows")]
fn install_menu(menu: &Menu, window: &Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = match window.window_handle() {
        Ok(h) => h,
        Err(_) => {
            log::warn!("[Debug] window handle unavailable; debug menu not installed");
            return;
        }
    };
    if let RawWindowHandle::Win32(h) = handle.as_raw() {
        let hwnd = h.hwnd.get();
        if let Err(e) = unsafe { menu.init_for_hwnd(hwnd) } {
            log::warn!("[Debug] init_for_hwnd failed: {e}");
        }
    } else {
        log::warn!("[Debug] non-Win32 window handle on Windows; menu not installed");
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn install_menu(_menu: &Menu, _window: &Window) {
    // Linux/other: muda requires GTK on Linux. The menu object exists so
    // `poll()` stays valid, but it isn't attached to a window. Use the
    // in-app keyboard shortcuts instead (e.g. Ctrl+Shift+M for the
    // material viewer).
}
