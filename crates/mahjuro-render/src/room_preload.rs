//! Background CPU decode for room GLBs, with GPU upload on the main thread when ready.
//!
//! Chain: main menu → shop → archive (hub); shop → hallway → gameplay (run).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use mahjuro_gfx_types::GraphicsMode;

use crate::loader_pool;

/// Next room in the run flow to decode on a worker thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoomSceneChain {
    Shop,
    /// Collection / Chronicle room — chained after shop finishes on the main menu hub.
    Archive,
    Hallway,
    Gameplay,
}

const PREFETCH_IDLE: u8 = 0;
const PREFETCH_IN_FLIGHT: u8 = 1;
const PREFETCH_DONE: u8 = 2;

static PREFETCH_GRAPHICS_MODE: AtomicU8 = AtomicU8::new(0);

pub(super) const SLOT_MAIN_MENU: u8 = 0;
pub(super) const SLOT_SHOP: u8 = 1;
pub(super) const SLOT_ARCHIVE: u8 = 2;
pub(super) const SLOT_HALLWAY: u8 = 3;
pub(super) const SLOT_GAMEPLAY: u8 = 4;

fn mark_prefetch_done(slot_id: u8) {
    let slot = match slot_id {
        SLOT_MAIN_MENU => &MAIN_MENU_PREFETCH,
        SLOT_SHOP => &SHOP_PREFETCH,
        SLOT_ARCHIVE => &ARCHIVE_PREFETCH,
        SLOT_HALLWAY => &HALLWAY_PREFETCH,
        SLOT_GAMEPLAY => &GAMEPLAY_PREFETCH,
        _ => return,
    };
    slot.state.store(PREFETCH_DONE, Ordering::Release);
}

struct PrefetchSlot {
    state: AtomicU8,
    id: u8,
}

impl PrefetchSlot {
    const fn new(id: u8) -> Self {
        Self {
            state: AtomicU8::new(PREFETCH_IDLE),
            id,
        }
    }

    fn try_start<F>(
        &self,
        profile_room: &'static str,
        already_ready: bool,
        priority_chain: bool,
        decode: F,
    ) where
        F: FnOnce() + Send + 'static,
    {
        if already_ready {
            self.try_drain();
            self.state.store(PREFETCH_DONE, Ordering::Release);
            return;
        }
        if !cpu_prefetch_may_start() {
            return;
        }
        if self.state.load(Ordering::Acquire) != PREFETCH_IDLE {
            return;
        }
        if self
            .state
            .compare_exchange(
                PREFETCH_IDLE,
                PREFETCH_IN_FLIGHT,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }
        let slot_id = self.id;
        let job = move || {
            let t0 = std::time::Instant::now();
            decode();
            if crate::room_gpu_profile::enabled() {
                crate::room_gpu_profile::record_cpu_decode(profile_room, t0.elapsed());
            }
            mark_prefetch_done(slot_id);
        };
        if priority_chain {
            loader_pool::submit_chain_prefetch(job);
        } else {
            loader_pool::submit_room_decode(job);
        }
        log::debug!("room preload: queued {profile_room} CPU decode");
    }

    fn try_drain(&self) {
        let _ = self.state.load(Ordering::Acquire);
    }

    fn join_blocking(&self) {
        while self.state.load(Ordering::Acquire) == PREFETCH_IN_FLIGHT {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

static MAIN_MENU_PREFETCH: PrefetchSlot = PrefetchSlot::new(SLOT_MAIN_MENU);
static SHOP_PREFETCH: PrefetchSlot = PrefetchSlot::new(SLOT_SHOP);
static ARCHIVE_PREFETCH: PrefetchSlot = PrefetchSlot::new(SLOT_ARCHIVE);
static HALLWAY_PREFETCH: PrefetchSlot = PrefetchSlot::new(SLOT_HALLWAY);
static GAMEPLAY_PREFETCH: PrefetchSlot = PrefetchSlot::new(SLOT_GAMEPLAY);

static GAMEPLAY_PREFETCH_LOGGED: OnceLock<()> = OnceLock::new();
static STAIRCASE_EAGER_CPU_QUEUED: AtomicBool = AtomicBool::new(false);

pub fn set_prefetch_graphics_mode(mode: GraphicsMode) {
    let tag = match mode {
        GraphicsMode::Performance => 1,
        GraphicsMode::LowMemory => 2,
        GraphicsMode::Visuals => 3,
    };
    PREFETCH_GRAPHICS_MODE.store(tag, Ordering::Relaxed);
}

fn prefetch_graphics_mode() -> GraphicsMode {
    match PREFETCH_GRAPHICS_MODE.load(Ordering::Relaxed) {
        1 => GraphicsMode::Performance,
        2 => GraphicsMode::LowMemory,
        _ => GraphicsMode::Visuals,
    }
}

pub fn max_concurrent_cpu_room_decodes(mode: GraphicsMode) -> usize {
    match mode {
        GraphicsMode::LowMemory => 1,
        GraphicsMode::Performance | GraphicsMode::Visuals => 2,
    }
}

fn room_holds_unreleased_cpu_meshes(decoded: bool, ready_for_gpu: bool) -> bool {
    decoded && ready_for_gpu
}

fn active_cpu_room_ram_residents() -> usize {
    let mut n = 0usize;
    for desc in crate::room_gpu_resident::RoomGpuResidentDesc::ALL {
        if room_holds_unreleased_cpu_meshes((desc.cpu_decoded)(), (desc.cpu_ready_for_gpu_upload)())
        {
            n += 1;
        }
    }
    for slot in [
        &MAIN_MENU_PREFETCH,
        &SHOP_PREFETCH,
        &ARCHIVE_PREFETCH,
        &HALLWAY_PREFETCH,
        &GAMEPLAY_PREFETCH,
    ] {
        if slot.state.load(Ordering::Acquire) == PREFETCH_IN_FLIGHT {
            n += 1;
        }
    }
    if STAIRCASE_EAGER_CPU_QUEUED.load(Ordering::Acquire) {
        n += 1;
    }
    n
}

fn cpu_prefetch_may_start() -> bool {
    active_cpu_room_ram_residents()
        < max_concurrent_cpu_room_decodes(prefetch_graphics_mode())
}

pub(super) fn reset_room_prefetch_slot(slot_id: u8) {
    let slot = match slot_id {
        SLOT_MAIN_MENU => &MAIN_MENU_PREFETCH,
        SLOT_SHOP => &SHOP_PREFETCH,
        SLOT_ARCHIVE => &ARCHIVE_PREFETCH,
        SLOT_HALLWAY => &HALLWAY_PREFETCH,
        SLOT_GAMEPLAY => &GAMEPLAY_PREFETCH,
        _ => return,
    };
    slot.state.store(PREFETCH_IDLE, Ordering::Release);
}

pub fn start_main_menu_cpu_prefetch() {
    MAIN_MENU_PREFETCH.try_start(
        "main_menu.glb",
        crate::main_menu_glb::main_menu_cpu_decoded(),
        false,
        crate::main_menu_glb::decode_main_menu_glb_into_cache,
    );
}

pub fn start_shop_cpu_prefetch() {
    SHOP_PREFETCH.try_start(
        "shop.glb",
        crate::room_glb::shop_cpu_decoded(),
        true,
        crate::room_glb::decode_shop_glb_into_cache,
    );
}

pub fn start_archive_cpu_prefetch() {
    ARCHIVE_PREFETCH.try_start(
        "archive.glb",
        crate::archive_glb::archive_cpu_decoded(),
        true,
        crate::archive_glb::decode_archive_glb_into_cache,
    );
}

pub fn advance_hub_cpu_prefetch_chain(_on_main_menu: bool) {
    if !crate::room_glb::shop_cpu_decoded() {
        return;
    }
    start_archive_cpu_prefetch();
}

/// Queue CPU decode for every hub/run room whose mesh cache is not ready yet.
///
/// Respects [`max_concurrent_cpu_room_decodes`]; call each frame so work continues
/// as in-flight decodes finish.
pub fn kick_eager_all_room_cpu_prefetches() {
    start_shop_cpu_prefetch();
    start_archive_cpu_prefetch();
    start_hallway_cpu_prefetch();
    start_gameplay_cpu_prefetch();
    kick_staircase_cpu_prefetch();
}

fn kick_staircase_cpu_prefetch() {
    if crate::staircase_glb::staircase_glb_loaded()
        || crate::staircase_glb::staircase_cpu_ready_for_gpu_upload()
    {
        STAIRCASE_EAGER_CPU_QUEUED.store(false, Ordering::Release);
        return;
    }
    if !cpu_prefetch_may_start() {
        return;
    }
    if STAIRCASE_EAGER_CPU_QUEUED.swap(true, Ordering::AcqRel) {
        return;
    }
    loader_pool::submit_chain_prefetch(|| {
        crate::staircase_glb::decode_staircase_glb_into_cache();
        STAIRCASE_EAGER_CPU_QUEUED.store(false, Ordering::Release);
    });
    log::debug!("room preload: queued staircase.glb CPU decode");
}

pub fn start_hallway_cpu_prefetch() {
    HALLWAY_PREFETCH.try_start(
        "hallway.glb",
        crate::hallway_glb::hallway_cpu_decoded(),
        true,
        crate::hallway_glb::decode_hallway_glb_into_cache,
    );
}

pub fn start_gameplay_cpu_prefetch() {
    GAMEPLAY_PREFETCH.try_start(
        "gameplay.glb",
        crate::gameplay_glb::gameplay_cpu_decoded(),
        true,
        || {
            crate::gameplay_glb::decode_gameplay_glb_into_cache();
            let state = crate::gameplay_glb::peek_gameplay_glb_load_state();
            if state != crate::gameplay_glb::GameplayGlbLoadState::Ready {
                GAMEPLAY_PREFETCH_LOGGED.get_or_init(|| {
                    log::warn!("gameplay.glb prefetch finished but room is not ready ({state:?})");
                });
            }
        },
    );
}

pub fn start_room_cpu_prefetch(scene: RoomSceneChain) {
    match scene {
        RoomSceneChain::Shop => start_shop_cpu_prefetch(),
        RoomSceneChain::Archive => start_archive_cpu_prefetch(),
        RoomSceneChain::Hallway => start_hallway_cpu_prefetch(),
        RoomSceneChain::Gameplay => start_gameplay_cpu_prefetch(),
    }
}

pub fn try_drain_room_cpu_prefetch_threads() {
    MAIN_MENU_PREFETCH.try_drain();
    SHOP_PREFETCH.try_drain();
    ARCHIVE_PREFETCH.try_drain();
    HALLWAY_PREFETCH.try_drain();
    GAMEPLAY_PREFETCH.try_drain();
}

pub fn join_main_menu_cpu_prefetch_blocking() {
    MAIN_MENU_PREFETCH.join_blocking();
}

pub fn join_shop_cpu_prefetch_blocking() {
    SHOP_PREFETCH.join_blocking();
}

pub fn join_archive_cpu_prefetch_blocking() {
    ARCHIVE_PREFETCH.join_blocking();
}

pub fn join_hallway_cpu_prefetch_blocking() {
    HALLWAY_PREFETCH.join_blocking();
}

pub fn join_gameplay_cpu_prefetch_blocking() {
    GAMEPLAY_PREFETCH.join_blocking();
}
