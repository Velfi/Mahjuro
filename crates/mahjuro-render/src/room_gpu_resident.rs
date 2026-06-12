//! Registry for hub/run room GLB GPU residency (bits, prefetch, CPU policy).
//!
//! New rooms should add one [`RoomGpuResidentDesc`] row and wire upload/eviction through
//! [`crate::wgpu_renderer::room_gpu_load`] — do not add ad-hoc `match` arms elsewhere.

use crate::scene_keys;

/// Stable GPU LRU / `rooms_gpu_loaded` bit for each room environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RoomGpuResidentId {
    Shop = 0,
    Hallway = 1,
    Staircase = 2,
    Archive = 3,
    Gameplay = 4,
    MainMenu = 5,
}

impl RoomGpuResidentId {
    pub const COUNT: usize = 6;

    pub const ALL: [Self; Self::COUNT] = [
        Self::Shop,
        Self::Hallway,
        Self::Staircase,
        Self::Archive,
        Self::Gameplay,
        Self::MainMenu,
    ];

    #[inline]
    pub const fn bit(self) -> u8 {
        1 << (self as u8)
    }

    #[inline]
    pub const fn desc(self) -> &'static RoomGpuResidentDesc {
        &RoomGpuResidentDesc::ALL[self as usize]
    }

    pub fn from_bit(bit: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.bit() == bit)
    }

    /// Scene key that pins this room during [`crate::wgpu_renderer::WgpuRenderer::poll_room_prefetch_gpu_uploads`].
    pub fn bit_for_scene_key(key: &str) -> Option<u8> {
        let k = scene_keys::normalize_scene_key(key);
        Self::ALL
            .into_iter()
            .find(|id| id.desc().matches_pin_scene_key(k))
            .map(Self::bit)
    }

    pub fn log_label(bit: u8) -> &'static str {
        Self::from_bit(bit)
            .map(|id| id.desc().log_label)
            .unwrap_or("unknown")
    }
}

/// Per-room policy row — keep upload/eviction behavior in sync across the engine.
pub struct RoomGpuResidentDesc {
    pub id: RoomGpuResidentId,
    /// e.g. `"shop.glb"` for logs and profiling.
    pub glb: &'static str,
    /// Short name for `gpu mem profile:` eviction lines.
    pub log_label: &'static str,
    pub startup_scope: &'static str,
    /// [`crate::room_preload`] worker slot, if any.
    pub prefetch_slot: Option<u8>,
    /// Counts toward the concurrent CPU decode cap when true.
    pub cpu_decoded: fn() -> bool,
    pub cpu_ready_for_gpu_upload: fn() -> bool,
    pub clear_cpu_cache: fn(),
    pub start_cpu_prefetch: fn(),
}

impl RoomGpuResidentDesc {
    pub const ALL: [Self; RoomGpuResidentId::COUNT] = [
        Self {
            id: RoomGpuResidentId::Shop,
            glb: "shop.glb",
            log_label: "shop",
            startup_scope: "wgpu.room.shop",
            prefetch_slot: Some(crate::room_preload::SLOT_SHOP),
            cpu_decoded: crate::room_glb::shop_cpu_decoded,
            cpu_ready_for_gpu_upload: crate::room_glb::shop_cpu_ready_for_gpu_upload,
            clear_cpu_cache: crate::room_glb::clear_shop_glb_cpu_cache,
            start_cpu_prefetch: crate::room_preload::start_shop_cpu_prefetch,
        },
        Self {
            id: RoomGpuResidentId::Hallway,
            glb: "hallway.glb",
            log_label: "hallway",
            startup_scope: "wgpu.room.hallway",
            prefetch_slot: Some(crate::room_preload::SLOT_HALLWAY),
            cpu_decoded: crate::hallway_glb::hallway_cpu_decoded,
            cpu_ready_for_gpu_upload: crate::hallway_glb::hallway_cpu_ready_for_gpu_upload,
            clear_cpu_cache: crate::hallway_glb::clear_hallway_glb_cpu_cache,
            start_cpu_prefetch: crate::room_preload::start_hallway_cpu_prefetch,
        },
        Self {
            id: RoomGpuResidentId::Staircase,
            glb: "staircase.glb",
            log_label: "staircase",
            startup_scope: "wgpu.room.staircase",
            prefetch_slot: None,
            cpu_decoded: crate::staircase_glb::staircase_glb_loaded,
            cpu_ready_for_gpu_upload: crate::staircase_glb::staircase_cpu_ready_for_gpu_upload,
            clear_cpu_cache: crate::staircase_glb::clear_staircase_glb_cpu_cache,
            start_cpu_prefetch: || {},
        },
        Self {
            id: RoomGpuResidentId::Archive,
            glb: "archive.glb",
            log_label: "archive",
            startup_scope: "wgpu.room.archive",
            prefetch_slot: Some(crate::room_preload::SLOT_ARCHIVE),
            cpu_decoded: crate::archive_glb::archive_cpu_decoded,
            cpu_ready_for_gpu_upload: crate::archive_glb::archive_cpu_ready_for_gpu_upload,
            clear_cpu_cache: crate::archive_glb::clear_archive_glb_cpu_cache,
            start_cpu_prefetch: crate::room_preload::start_archive_cpu_prefetch,
        },
        Self {
            id: RoomGpuResidentId::Gameplay,
            glb: "gameplay.glb",
            log_label: "gameplay",
            startup_scope: "wgpu.room.gameplay",
            prefetch_slot: Some(crate::room_preload::SLOT_GAMEPLAY),
            cpu_decoded: crate::gameplay_glb::gameplay_cpu_decoded,
            cpu_ready_for_gpu_upload: crate::gameplay_glb::gameplay_cpu_ready_for_gpu_upload,
            clear_cpu_cache: crate::gameplay_glb::clear_gameplay_glb_cpu_cache,
            start_cpu_prefetch: crate::room_preload::start_gameplay_cpu_prefetch,
        },
        Self {
            id: RoomGpuResidentId::MainMenu,
            glb: "main_menu.glb",
            log_label: "main_menu",
            startup_scope: "wgpu.room.main_menu",
            prefetch_slot: Some(crate::room_preload::SLOT_MAIN_MENU),
            cpu_decoded: crate::main_menu_glb::main_menu_cpu_decoded,
            cpu_ready_for_gpu_upload: crate::main_menu_glb::main_menu_cpu_ready_for_gpu_upload,
            clear_cpu_cache: crate::main_menu_glb::clear_main_menu_glb_cpu_cache,
            start_cpu_prefetch: crate::room_preload::start_main_menu_cpu_prefetch,
        },
    ];

    pub fn from_bit(bit: u8) -> Option<&'static Self> {
        RoomGpuResidentId::from_bit(bit).map(RoomGpuResidentId::desc)
    }

    #[inline]
    pub const fn bit(&self) -> u8 {
        self.id.bit()
    }

    fn matches_pin_scene_key(&self, key: &str) -> bool {
        match self.id {
            RoomGpuResidentId::MainMenu => key == scene_keys::MAIN_MENU,
            RoomGpuResidentId::Shop => key == scene_keys::SHOP,
            RoomGpuResidentId::Hallway => key == scene_keys::HALLWAY,
            RoomGpuResidentId::Staircase => key == scene_keys::STAIRWAY,
            RoomGpuResidentId::Archive => key == scene_keys::ARCHIVE,
            RoomGpuResidentId::Gameplay => {
                matches!(key, scene_keys::GAMEPLAY | scene_keys::DEFEAT)
            }
        }
    }

    pub fn clear_cpu_cache_for_gpu_evict(bit: u8) {
        if let Some(d) = Self::from_bit(bit) {
            (d.clear_cpu_cache)();
        }
    }

    pub fn restart_cpu_after_gpu_miss(bit: u8) {
        let Some(d) = Self::from_bit(bit) else {
            return;
        };
        if let Some(slot) = d.prefetch_slot {
            crate::room_preload::reset_room_prefetch_slot(slot);
        }
        (d.start_cpu_prefetch)();
    }
}

/// Victory run-summary 3D moon (`MoonObject` from `main_menu.glb`) vs procedural 2D disc.
#[inline]
pub fn victory_uses_3d_moon(mode: mahjuro_gfx_types::GraphicsMode) -> bool {
    mode != mahjuro_gfx_types::GraphicsMode::LowMemory
}

/// Scene key → pinned room GPU bit, accounting for graphics preset policy.
pub fn room_gpu_bit_for_scene_key(
    key: &str,
    graphics_mode: mahjuro_gfx_types::GraphicsMode,
) -> Option<u8> {
    let key = scene_keys::normalize_scene_key(key);
    if key == scene_keys::VICTORY {
        return victory_uses_3d_moon(graphics_mode).then(|| RoomGpuResidentId::MainMenu.bit());
    }
    RoomGpuResidentId::bit_for_scene_key(key)
}

/// Legacy bit constants — prefer [`RoomGpuResidentId::bit`].
pub const ROOM_SHOP: u8 = RoomGpuResidentId::Shop.bit();
pub const ROOM_HALLWAY: u8 = RoomGpuResidentId::Hallway.bit();
pub const ROOM_STAIRCASE: u8 = RoomGpuResidentId::Staircase.bit();
pub const ROOM_ARCHIVE: u8 = RoomGpuResidentId::Archive.bit();
pub const ROOM_GAMEPLAY: u8 = RoomGpuResidentId::Gameplay.bit();
pub const ROOM_MAIN_MENU: u8 = RoomGpuResidentId::MainMenu.bit();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_bits_are_unique_powers_of_two() {
        let mut seen = 0u8;
        for id in RoomGpuResidentId::ALL {
            let b = id.bit();
            assert_eq!(b.count_ones(), 1);
            assert_eq!(seen & b, 0);
            seen |= b;
        }
    }

    #[test]
    fn desc_table_matches_enum_order() {
        for (i, id) in RoomGpuResidentId::ALL.into_iter().enumerate() {
            assert_eq!(RoomGpuResidentDesc::ALL[i].id, id);
            assert_eq!(RoomGpuResidentDesc::ALL[i].bit(), id.bit());
        }
    }

    #[test]
    fn bit_for_scene_key_maps_legacy_slugs() {
        assert_eq!(
            RoomGpuResidentId::bit_for_scene_key("main_menu_exterior"),
            Some(ROOM_MAIN_MENU)
        );
        assert_eq!(
            RoomGpuResidentId::bit_for_scene_key("pick_chamber"),
            Some(ROOM_HALLWAY)
        );
        assert_eq!(
            RoomGpuResidentId::bit_for_scene_key("game_over"),
            Some(ROOM_GAMEPLAY)
        );
        assert_eq!(RoomGpuResidentId::bit_for_scene_key("victory"), None);
        assert_eq!(
            room_gpu_bit_for_scene_key("victory", mahjuro_gfx_types::GraphicsMode::Visuals),
            Some(ROOM_MAIN_MENU)
        );
        assert_eq!(
            room_gpu_bit_for_scene_key("victory", mahjuro_gfx_types::GraphicsMode::LowMemory),
            None
        );
        assert_eq!(
            room_gpu_bit_for_scene_key("gameplay", mahjuro_gfx_types::GraphicsMode::LowMemory),
            Some(ROOM_GAMEPLAY)
        );
        assert_eq!(
            room_gpu_bit_for_scene_key("showcase", mahjuro_gfx_types::GraphicsMode::LowMemory),
            Some(ROOM_SHOP)
        );
        assert_eq!(
            room_gpu_bit_for_scene_key("tutorial", mahjuro_gfx_types::GraphicsMode::LowMemory),
            Some(ROOM_GAMEPLAY)
        );
    }
}
