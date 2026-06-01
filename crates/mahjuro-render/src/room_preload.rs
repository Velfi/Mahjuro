//! Background CPU decode for room GLBs, with GPU upload on the main thread when ready.
//!
//! Chain: main menu → shop → archive (hub); shop → hallway → gameplay (run).

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;
use std::thread::JoinHandle;

use parking_lot::Mutex;

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

struct PrefetchSlot {
    state: AtomicU8,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// Set while a worker is in flight so `join_blocking` can skip self-join (macOS EDEADLK).
    worker_tid: Mutex<Option<std::thread::ThreadId>>,
}

impl PrefetchSlot {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(PREFETCH_IDLE),
            thread: Mutex::new(None),
            worker_tid: Mutex::new(None),
        }
    }

    fn try_start<F>(
        &self,
        thread_name: &'static str,
        profile_room: &'static str,
        already_ready: bool,
        decode: F,
    ) where
        F: FnOnce() + Send + 'static,
    {
        if already_ready {
            // Decode may have finished on a worker while state is still IN_FLIGHT — join it
            // before marking DONE so we never orphan a live thread handle.
            self.try_drain();
            self.state.store(PREFETCH_DONE, Ordering::Release);
            return;
        }
        if self.state.load(Ordering::Acquire) != PREFETCH_IDLE {
            return;
        }
        if self
            .state
            .compare_exchange(PREFETCH_IDLE, PREFETCH_IN_FLIGHT, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let handle = std::thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || {
                let t0 = std::time::Instant::now();
                decode();
                if crate::room_gpu_profile::enabled() {
                    crate::room_gpu_profile::record_cpu_decode(profile_room, t0.elapsed());
                }
                // Slot state is set to DONE after join in `try_drain`.
            })
            .unwrap_or_else(|e| panic!("failed to spawn {thread_name} prefetch thread: {e}"));
        *self.worker_tid.lock() = Some(handle.thread().id());
        *self.thread.lock() = Some(handle);
        log::debug!("room preload: started {profile_room} CPU decode");
    }

    fn try_drain(&self) {
        let mut guard = self.thread.lock();
        let Some(handle) = guard.as_ref() else {
            return;
        };
        if !handle.is_finished() {
            return;
        }
        let handle = guard.take().expect("prefetch handle");
        drop(guard);
        if let Err(e) = handle.join() {
            log::error!("room preload thread panicked: {e:?}");
        }
        *self.worker_tid.lock() = None;
        self.state.store(PREFETCH_DONE, Ordering::Release);
    }

    fn join_blocking(&self) {
        if self
            .worker_tid
            .lock()
            .is_some_and(|tid| tid == std::thread::current().id())
        {
            return;
        }
        self.try_drain();
        if self.state.load(Ordering::Acquire) == PREFETCH_IN_FLIGHT {
            let handle = self.thread.lock().take();
            if let Some(handle) = handle
                && let Err(e) = handle.join() {
                    log::error!("room preload thread panicked: {e:?}");
                }
            *self.worker_tid.lock() = None;
            self.state.store(PREFETCH_DONE, Ordering::Release);
        }
    }
}

static MAIN_MENU_PREFETCH: PrefetchSlot = PrefetchSlot::new();
static SHOP_PREFETCH: PrefetchSlot = PrefetchSlot::new();
static ARCHIVE_PREFETCH: PrefetchSlot = PrefetchSlot::new();
static HALLWAY_PREFETCH: PrefetchSlot = PrefetchSlot::new();
static GAMEPLAY_PREFETCH: PrefetchSlot = PrefetchSlot::new();

static GAMEPLAY_PREFETCH_LOGGED: OnceLock<()> = OnceLock::new();

/// Start decoding `main_menu.glb` on a worker thread (idempotent).
pub fn start_main_menu_cpu_prefetch() {
    MAIN_MENU_PREFETCH.try_start(
        "mahjuro-main-menu-glb",
        "main_menu.glb",
        crate::main_menu_glb::main_menu_cpu_decoded(),
        crate::main_menu_glb::decode_main_menu_glb_into_cache,
    );
}

/// Start decoding `shop.glb` on a worker thread (idempotent).
pub fn start_shop_cpu_prefetch() {
    SHOP_PREFETCH.try_start(
        "mahjuro-shop-glb",
        "shop.glb",
        crate::room_glb::shop_cpu_decoded(),
        crate::room_glb::decode_shop_glb_into_cache,
    );
}

/// Start decoding `archive.glb` on a worker thread (idempotent).
pub fn start_archive_cpu_prefetch() {
    ARCHIVE_PREFETCH.try_start(
        "mahjuro-archive-glb",
        "archive.glb",
        crate::archive_glb::archive_cpu_decoded(),
        crate::archive_glb::decode_archive_glb_into_cache,
    );
}

/// Hub chain: once `shop.glb` CPU decode is in cache, start `archive.glb` on a worker.
/// GPU uploads stay in [`crate::wgpu_renderer::room_gpu_load`] per scene.
pub fn advance_hub_cpu_prefetch_chain() {
    if !crate::room_glb::shop_cpu_decoded() {
        return;
    }
    start_archive_cpu_prefetch();
}

/// Start decoding `hallway.glb` on a worker thread (idempotent).
pub fn start_hallway_cpu_prefetch() {
    HALLWAY_PREFETCH.try_start(
        "mahjuro-hallway-glb",
        "hallway.glb",
        crate::hallway_glb::hallway_cpu_decoded(),
        crate::hallway_glb::decode_hallway_glb_into_cache,
    );
}

/// Start decoding `gameplay.glb` on a worker thread (idempotent).
pub fn start_gameplay_cpu_prefetch() {
    GAMEPLAY_PREFETCH.try_start(
        "mahjuro-gameplay-glb",
        "gameplay.glb",
        crate::gameplay_glb::gameplay_cpu_decoded(),
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

/// Join any finished prefetch workers without blocking on in-flight work.
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
