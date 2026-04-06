//! Native OS menubar "Debug" menu using muda.
//!
//! Provides debug shortcuts for setting player level, gold, relics, and card inventory.

use muda::{Menu, MenuEvent, MenuId, MenuItem, Submenu};

use crate::core::relic::{RelicId, all_relic_defs};

/// Identifies which debug action was triggered.
#[derive(Clone, Debug)]
pub enum DebugAction {
    SetLevel(u32),
    SetGold(u32),
    AddRelic(RelicId),
    ClearRelics,
    SetCardInventoryStandard,
    SetCardInventoryNumbersOnly,
    SetCardInventoryHonorsOnly,
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

        // Card Inventory submenu.
        let card_sub = Submenu::new("Card Inventory", true);
        let std_item = MenuItem::new("Standard (136 tiles)", true, None);
        mappings.push((std_item.id().clone(), DebugAction::SetCardInventoryStandard));
        let _ = card_sub.append(&std_item);

        let num_item = MenuItem::new("Numbers Only (no honors)", true, None);
        mappings.push((num_item.id().clone(), DebugAction::SetCardInventoryNumbersOnly));
        let _ = card_sub.append(&num_item);

        let hon_item = MenuItem::new("Honors Only (winds + dragons)", true, None);
        mappings.push((hon_item.id().clone(), DebugAction::SetCardInventoryHonorsOnly));
        let _ = card_sub.append(&hon_item);

        let _ = debug_menu.append(&card_sub);

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
