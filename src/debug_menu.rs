// When the debug menu is gated out (release builds without
// `MAHJURO_DEBUG_MENU=1`), only [`DebugAction`] is kept for keyboard routing.

//! Native OS menubar "Debug" menu using muda.
//!
//! Provides debug shortcuts grouped by purpose: overlays, tuning panels,
//! profiling, in-run cheats, standalone labs, end screens, and profile meta.
//!
//! ## Cross-platform installation
//!
//! - **macOS**: installed as the global NSApp menu via `init_for_nsapp()`.
//! - **Windows**: installed onto the main window's HWND via `init_for_hwnd()`.
//!   The HWND is extracted from the SDL window's `RawWindowHandle::Win32`.
//! - **Linux**: muda requires GTK on Linux, which the rest of the app does not
//!   pull in. The menu is built (so `poll()` continues to work for any
//!   programmatically-injected events) but is *not* attached to the window —
//!   Linux users reach debug actions via the Debug menu (or Ctrl/Cmd+Shift+H /
//!   Ctrl/Cmd+Shift+T when `debug_menu_enabled`).

#[cfg(debug_menu_enabled)]
use muda::accelerator::{Accelerator, Code, Modifiers, CMD_OR_CTRL};
#[cfg(debug_menu_enabled)]
use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
#[cfg(all(debug_menu_enabled, target_os = "windows"))]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[cfg(debug_menu_enabled)]
use sdl3::keyboard::{Mod, Scancode};
#[cfg(debug_menu_enabled)]
use sdl3::video::Window;

use crate::core::ordeal::OrdealKind;
#[cfg(debug_menu_enabled)]
use crate::core::ordeal::{all_ordeals, final_ordeals};
use crate::core::relic::RelicId;
#[cfg(debug_menu_enabled)]
use crate::core::relic::all_relic_defs;
use crate::core::talisman::TalismanKind;
use crate::core::tile::Suit;
#[cfg(debug_menu_enabled)]
use crate::core::tile::Tile;
use crate::core::zodiac::ZodiacKind;

/// Identifies which debug action was triggered.
#[derive(Clone, Debug)]
pub enum DebugAction {
    SetLevel(u32),
    SetYen(u32),
    AddRelic(RelicId),
    ClearRelics,
    AddTalisman(TalismanKind),
    AddZodiac(ZodiacKind),
    ClearConsumables,
    /// Replace the current ante's `upcoming_ordeal` and re-resolve its effect
    /// (so reactive variants like Mirror/Tax Collector pick fresh).
    SetOrdeal(OrdealKind),
    /// Replace the current dora indicator with a single tile of the given
    /// face. Clobbers any extra dora revealed by Dora Crown.
    SetDora(Suit, u8),
    ToggleShowFps,
    /// Hide all 2D UI draw commands so only the 3D scene renders.
    ToggleHide2dUi,
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
    /// Per-scene tonemap / post-FX + room GLB lighting (right panel).
    OpenSceneLookDebug,
    /// Main-menu moon / rain / moths tuning overlay.
    OpenRainDebug,
    /// Pick-blind hallway vertex warp (sliders; Ctrl+C copies a Rust snapshot).
    OpenHallwayHallFxDebug,
    /// Shop / gameplay candle flame shader + placement tuning.
    OpenFlameDebug,
    /// Capture GPU pass timings (shadow, main, room-bloom, GI, bloom, tonemap,
    /// …) averaged over the next 100 rendered frames and log the result. Only
    /// meaningful on backends that support `wgpu::Features::TIMESTAMP_QUERY`.
    /// The synchronous readback
    /// inflates the CPU `render` stage if the CPU profile runs at the same
    /// time, so this is split from [`DebugAction::ProfileCpu`].
    ProfileGpu,
    /// Capture CPU stage timings (`update`, `draw_frame`, `render`) averaged
    /// over the next 100 rendered frames and log the result. Always
    /// available; safe to run while the GPU profile is *not* active.
    ProfileCpu,
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
    /// Push the material viewer pushdown scene onto the overlay stack.
    /// Shows one preview orb per `MaterialKind` for visual inspection.
    OpenMaterialViewer,
    /// Push the transition playground scene onto the overlay stack.
    OpenTransitionPlayground,
    OpenAnimationLab,
    OpenRollerLab,
    OpenCascadeLab,
    OpenRumbleLab,
    OpenShadowAoLab,
    OpenTileAnchorLab,
    OpenButtonAabbLab,
    OpenTixels,
    /// Open a simple in-app About modal. Used on macOS to avoid the native
    /// About panel's icon conversion path in `muda`.
    #[cfg(target_os = "macos")]
    OpenAbout,
    /// Meta: discover all transformation successors; ensure progression lists
    /// all fragile primaries. Run: mark every burn chain as extinct so
    /// successors can appear in the shop pool.
    UnlockAllTransformationsAndSuccessors,
    /// Play N headless bot runs and append each terminal state to
    /// `run_history` + career counters (Chronicle debug fill).
    SeedChronicleFromBotRuns(u32),
    /// Mark Kokushi Musō as discovered in the active profile (journal, guide, Qilin ribbon).
    RevealKokushiMusou,
    /// Pick-blind hallway: cycle seed/tint at accelerating intervals (~5 s).
    TriggerTrailerMode,
    /// Meta: unlock every tile material and every season on every material.
    UnlockAllTilesetsAndSeasons,
}

#[cfg(debug_menu_enabled)]
fn accel_cmd_shift(key: Code) -> Accelerator {
    Accelerator::new(Some(CMD_OR_CTRL | Modifiers::SHIFT), key)
}

/// SDL fallback when the native menubar does not receive the chord (Linux).
/// macOS/Windows also register these on the matching [`MenuItem`] accelerators.
#[cfg(debug_menu_enabled)]
pub fn debug_action_for_keyboard_shortcut(
    scancode: Option<Scancode>,
    keymod: Mod,
    repeat: bool,
) -> Option<DebugAction> {
    if repeat {
        return None;
    }
    let shift = keymod.contains(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
    let cmd_or_ctrl = keymod.contains(
        Mod::LCTRLMOD | Mod::RCTRLMOD | Mod::LGUIMOD | Mod::RGUIMOD,
    );
    if !shift || !cmd_or_ctrl {
        return None;
    }
    match scancode {
        Some(Scancode::H) => Some(DebugAction::ToggleHide2dUi),
        Some(Scancode::T) => Some(DebugAction::TriggerTrailerMode),
        _ => None,
    }
}

/// Holds the menu bar and maps MenuIds to DebugActions.
#[cfg(debug_menu_enabled)]
pub struct DebugMenuBar {
    // Must be retained for the lifetime of the installed native menubar.
    // On macOS, AppKit can invoke menu item actions long after
    // `init_for_nsapp()`, so dropping the Rust-side menu object leaves the
    // native callbacks dangling.
    _retained_menu: Menu,
    mappings: Vec<(MenuId, DebugAction)>,
}

#[cfg(debug_menu_enabled)]
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

        // ── Overlays ─────────────────────────────────────────────────────────
        // On-screen HUD toggles, visibility panels, and overlay smoke tests.
        let overlays_sub = Submenu::new("Overlays", true);

        let fps_item = MenuItem::new("Show FPS", true, None);
        mappings.push((fps_item.id().clone(), DebugAction::ToggleShowFps));
        let _ = overlays_sub.append(&fps_item);

        let hide_2d_ui_item = MenuItem::new(
            "Hide 2D UI",
            true,
            Some(accel_cmd_shift(Code::KeyH)),
        );
        mappings.push((hide_2d_ui_item.id().clone(), DebugAction::ToggleHide2dUi));
        let _ = overlays_sub.append(&hide_2d_ui_item);

        let visibility_item = MenuItem::new("Gameplay Visibility...", true, None);
        mappings.push((
            visibility_item.id().clone(),
            DebugAction::OpenDebugVisibility,
        ));
        let _ = overlays_sub.append(&visibility_item);

        let axes_item = MenuItem::new("World Axes", true, None);
        mappings.push((axes_item.id().clone(), DebugAction::ToggleWorldAxes));
        let _ = overlays_sub.append(&axes_item);

        let test_overlay_item = MenuItem::new("Test Overlay", true, None);
        mappings.push((test_overlay_item.id().clone(), DebugAction::TestOverlay));
        let _ = overlays_sub.append(&test_overlay_item);

        let _ = debug_menu.append(&overlays_sub);

        // ── Tuning ───────────────────────────────────────────────────────────
        // Live parameter panels. Keep entries one level deep for faster access.
        let tuning_sub = Submenu::new("Tuning", true);
        let cascade_tuning_item = MenuItem::new("Cascade Tuning...", true, None);
        mappings.push((cascade_tuning_item.id().clone(), DebugAction::OpenTuning));
        let _ = tuning_sub.append(&cascade_tuning_item);
        let _ = tuning_sub.append(&PredefinedMenuItem::separator());

        let camera_item = MenuItem::new("Camera...", true, None);
        mappings.push((camera_item.id().clone(), DebugAction::OpenCameraDebug));
        let _ = tuning_sub.append(&camera_item);
        let scene_look_item = MenuItem::new("Scene Look (per scene)...", true, None);
        mappings.push((
            scene_look_item.id().clone(),
            DebugAction::OpenSceneLookDebug,
        ));
        let _ = tuning_sub.append(&scene_look_item);
        let _ = tuning_sub.append(&PredefinedMenuItem::separator());

        let rain_item = MenuItem::new("Main Menu Effects...", true, None);
        mappings.push((rain_item.id().clone(), DebugAction::OpenRainDebug));
        let _ = tuning_sub.append(&rain_item);
        let flame_item = MenuItem::new("Candle Flames (shop / gameplay)...", true, None);
        mappings.push((flame_item.id().clone(), DebugAction::OpenFlameDebug));
        let _ = tuning_sub.append(&flame_item);
        let hall_fx_item = MenuItem::new("Hallway Vertex Warp...", true, None);
        mappings.push((
            hall_fx_item.id().clone(),
            DebugAction::OpenHallwayHallFxDebug,
        ));
        let _ = tuning_sub.append(&hall_fx_item);
        let _ = tuning_sub.append(&PredefinedMenuItem::separator());

        let sfx_item = MenuItem::new("Sound Effects Test...", true, None);
        mappings.push((sfx_item.id().clone(), DebugAction::OpenSfxTest));
        let _ = tuning_sub.append(&sfx_item);

        let _ = debug_menu.append(&tuning_sub);

        // ── Profiling ────────────────────────────────────────────────────────
        let profiling_sub = Submenu::new("Profiling", true);

        let profile_cpu_item = MenuItem::new("CPU (100 frames)", true, None);
        mappings.push((profile_cpu_item.id().clone(), DebugAction::ProfileCpu));
        let _ = profiling_sub.append(&profile_cpu_item);

        let profile_gpu_item = MenuItem::new("GPU (100 frames)", true, None);
        mappings.push((profile_gpu_item.id().clone(), DebugAction::ProfileGpu));
        let _ = profiling_sub.append(&profile_gpu_item);

        let _ = debug_menu.append(&profiling_sub);

        // ── Run ──────────────────────────────────────────────────────────────
        // In-run shortcuts: shop, FX demos, and direct state overrides.
        let run_sub = Submenu::new("Run", true);

        let reroll_item = MenuItem::new("Reroll Shop", true, None);
        mappings.push((reroll_item.id().clone(), DebugAction::RerollShop));
        let _ = run_sub.append(&reroll_item);

        let open_pack_item = MenuItem::new("Open Tile Pack", true, None);
        mappings.push((open_pack_item.id().clone(), DebugAction::OpenPack));
        let _ = run_sub.append(&open_pack_item);

        let demo_cascade_item = MenuItem::new("Demo Score Cascade", true, None);
        mappings.push((demo_cascade_item.id().clone(), DebugAction::DemoCascade));
        let _ = run_sub.append(&demo_cascade_item);

        let _ = run_sub.append(&PredefinedMenuItem::separator());

        // Direct "set" entries avoid extra submenu drilling.
        for &amount in &[0u32, 10, 50, 100, 500, 9999] {
            let item = MenuItem::new(format!("Set Gold: {amount}"), true, None);
            mappings.push((item.id().clone(), DebugAction::SetYen(amount)));
            let _ = run_sub.append(&item);
        }
        let _ = run_sub.append(&PredefinedMenuItem::separator());

        for lvl in 1..=14u32 {
            let item = MenuItem::new(
                format!(
                    "Set Player Depth: {}",
                    crate::core::progression::meta_depth_roman(lvl)
                ),
                true,
                None,
            );
            mappings.push((item.id().clone(), DebugAction::SetLevel(lvl)));
            let _ = run_sub.append(&item);
        }
        let _ = run_sub.append(&PredefinedMenuItem::separator());

        let boss_sub = Submenu::new("Set Current Boss", true);
        for def in all_ordeals().iter().chain(final_ordeals().iter()) {
            let item = MenuItem::new(def.name, true, None);
            mappings.push((item.id().clone(), DebugAction::SetOrdeal(def.kind)));
            let _ = boss_sub.append(&item);
        }
        let _ = run_sub.append(&boss_sub);

        let dora_sub = Submenu::new("Set Dora", true);
        for (suit_label, suit, max_rank) in [
            ("Manzu (m)", Suit::Manzu, 9u8),
            ("Souzu (s)", Suit::Souzu, 9u8),
            ("Pinzu (p)", Suit::Pinzu, 9u8),
            ("Winds", Suit::Wind, 4u8),
            ("Dragons", Suit::Dragon, 3u8),
        ] {
            for rank in 1..=max_rank {
                let face = Tile::new(suit, rank, 0).full_name();
                let item = MenuItem::new(format!("{suit_label}: {face}"), true, None);
                mappings.push((item.id().clone(), DebugAction::SetDora(suit, rank)));
                let _ = dora_sub.append(&item);
            }
            if !matches!(suit, Suit::Dragon) {
                let _ = dora_sub.append(&PredefinedMenuItem::separator());
            }
        }
        let _ = run_sub.append(&dora_sub);

        let clear_item = MenuItem::new("Clear All Relics", true, None);
        mappings.push((clear_item.id().clone(), DebugAction::ClearRelics));
        let _ = run_sub.append(&clear_item);
        let add_sub = Submenu::new("Add Relic", true);
        for def in all_relic_defs() {
            let item = MenuItem::new(def.name, true, None);
            mappings.push((item.id().clone(), DebugAction::AddRelic(def.id)));
            let _ = add_sub.append(&item);
        }
        let _ = run_sub.append(&add_sub);

        let _ = run_sub.append(&PredefinedMenuItem::separator());

        let clear_cons_item = MenuItem::new("Clear All Consumables", true, None);
        mappings.push((clear_cons_item.id().clone(), DebugAction::ClearConsumables));
        let _ = run_sub.append(&clear_cons_item);
        let add_talisman_sub = Submenu::new("Add Talisman", true);
        for &kind in TalismanKind::all() {
            let item = MenuItem::new(kind.name(), true, None);
            mappings.push((item.id().clone(), DebugAction::AddTalisman(kind)));
            let _ = add_talisman_sub.append(&item);
        }
        let _ = run_sub.append(&add_talisman_sub);
        let add_zodiac_sub = Submenu::new("Add Zodiac", true);
        for &kind in ZodiacKind::all() {
            let item = MenuItem::new(kind.name(), true, None);
            mappings.push((item.id().clone(), DebugAction::AddZodiac(kind)));
            let _ = add_zodiac_sub.append(&item);
        }
        let _ = run_sub.append(&add_zodiac_sub);

        let _ = debug_menu.append(&run_sub);

        let _ = debug_menu.append(&PredefinedMenuItem::separator());

        // ── Labs (direct entries) ────────────────────────────────────────────
        // Standalone pushdown scenes for layout, animation, and asset review.
        let animation_lab_item = MenuItem::new("Animation Lab...", true, None);
        mappings.push((
            animation_lab_item.id().clone(),
            DebugAction::OpenAnimationLab,
        ));
        let _ = debug_menu.append(&animation_lab_item);

        let button_aabb_lab_item = MenuItem::new("Button AABB Lab...", true, None);
        mappings.push((
            button_aabb_lab_item.id().clone(),
            DebugAction::OpenButtonAabbLab,
        ));
        let _ = debug_menu.append(&button_aabb_lab_item);

        let material_viewer_item = MenuItem::new("Material Viewer...", true, None);
        mappings.push((
            material_viewer_item.id().clone(),
            DebugAction::OpenMaterialViewer,
        ));
        let _ = debug_menu.append(&material_viewer_item);

        let roller_lab_item = MenuItem::new("Roller Lab...", true, None);
        mappings.push((roller_lab_item.id().clone(), DebugAction::OpenRollerLab));
        let _ = debug_menu.append(&roller_lab_item);

        let cascade_lab_item = MenuItem::new("Cascade Lab...", true, None);
        mappings.push((cascade_lab_item.id().clone(), DebugAction::OpenCascadeLab));
        let _ = debug_menu.append(&cascade_lab_item);

        let rumble_lab_item = MenuItem::new("Rumble Lab...", true, None);
        mappings.push((rumble_lab_item.id().clone(), DebugAction::OpenRumbleLab));
        let _ = debug_menu.append(&rumble_lab_item);

        let shadow_ao_lab_item = MenuItem::new("Shadow & AO Lab...", true, None);
        mappings.push((
            shadow_ao_lab_item.id().clone(),
            DebugAction::OpenShadowAoLab,
        ));
        let _ = debug_menu.append(&shadow_ao_lab_item);

        let tile_anchor_lab_item = MenuItem::new("Tile Anchor Lab...", true, None);
        mappings.push((
            tile_anchor_lab_item.id().clone(),
            DebugAction::OpenTileAnchorLab,
        ));
        let _ = debug_menu.append(&tile_anchor_lab_item);

        let tixels_item = MenuItem::new("Tixels...", true, None);
        mappings.push((tixels_item.id().clone(), DebugAction::OpenTixels));
        let _ = debug_menu.append(&tixels_item);

        let transition_playground_item = MenuItem::new("Transition Playground...", true, None);
        mappings.push((
            transition_playground_item.id().clone(),
            DebugAction::OpenTransitionPlayground,
        ));
        let _ = debug_menu.append(&transition_playground_item);

        let _ = debug_menu.append(&PredefinedMenuItem::separator());

        // ── Screens (direct entries) ─────────────────────────────────────────
        // Jump to end-of-run presentation without playing through.
        let victory_item = MenuItem::new("Victory", true, None);
        mappings.push((victory_item.id().clone(), DebugAction::ShowVictoryScreen));
        let _ = debug_menu.append(&victory_item);

        let defeat_item = MenuItem::new("Defeat", true, None);
        mappings.push((defeat_item.id().clone(), DebugAction::ShowDefeatScreen));
        let _ = debug_menu.append(&defeat_item);

        let trailer_item = MenuItem::new(
            "Trigger Trailer Mode",
            true,
            Some(accel_cmd_shift(Code::KeyT)),
        );
        mappings.push((trailer_item.id().clone(), DebugAction::TriggerTrailerMode));
        let _ = debug_menu.append(&trailer_item);
        let _ = debug_menu.append(&PredefinedMenuItem::separator());

        // ── Profile ──────────────────────────────────────────────────────────
        // Persistent meta / career cheats (saved to the active profile).
        let profile_sub = Submenu::new("Profile", true);

        let unlock_transforms_item =
            MenuItem::new("Unlock Transformations & Successors", true, None);
        mappings.push((
            unlock_transforms_item.id().clone(),
            DebugAction::UnlockAllTransformationsAndSuccessors,
        ));
        let _ = profile_sub.append(&unlock_transforms_item);

        let reveal_kokushi_item = MenuItem::new("Reveal Kokushi Musō", true, None);
        mappings.push((
            reveal_kokushi_item.id().clone(),
            DebugAction::RevealKokushiMusou,
        ));
        let _ = profile_sub.append(&reveal_kokushi_item);

        let unlock_tilesets_item = MenuItem::new("Unlock All Tilesets & Seasons", true, None);
        mappings.push((
            unlock_tilesets_item.id().clone(),
            DebugAction::UnlockAllTilesetsAndSeasons,
        ));
        let _ = profile_sub.append(&unlock_tilesets_item);

        let chronicle_sub = Submenu::new("Seed Chronicle (bot runs)", true);
        for &n in &[5u32, 15, 30, 60] {
            let item = MenuItem::new(format!("{n} bot runs"), true, None);
            mappings.push((item.id().clone(), DebugAction::SeedChronicleFromBotRuns(n)));
            let _ = chronicle_sub.append(&item);
        }
        let _ = profile_sub.append(&chronicle_sub);

        let _ = debug_menu.append(&profile_sub);

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
#[cfg(all(debug_menu_enabled, target_os = "macos"))]
fn install_menu(menu: &Menu, _window: &Window) {
    menu.init_for_nsapp();
}

#[cfg(all(debug_menu_enabled, target_os = "windows"))]
fn install_menu(menu: &Menu, window: &Window) {
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

#[cfg(all(debug_menu_enabled, not(any(target_os = "macos", target_os = "windows"))))]
fn install_menu(_menu: &Menu, _window: &Window) {
    // Linux/other: muda requires GTK on Linux. The menu object exists so
    // `poll()` stays valid, but it isn't attached to a window. Use the
    // in-app keyboard shortcuts instead (Ctrl/Cmd+Shift+H and +T when
    // `debug_menu_enabled`).
}
