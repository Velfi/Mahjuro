//! GPU memory pressure classification for eager room preload gating.
//!
//! Uses wgpu allocator totals when available (Vulkan/DX12); falls back to resident
//! count vs the active [`mahjuro_gfx_types::GraphicsMode`] cap on Metal/GLES.

use std::sync::{
    OnceLock,
    atomic::{AtomicU8, AtomicU32, Ordering},
};

/// Pressure tier for eager (non-scene-critical) room GPU warm-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum GpuMemoryPressure {
    #[default]
    Normal = 0,
    /// Pause optional eager warm-up (hallway/gameplay/staircase); hub rooms still allowed.
    Constrained = 1,
    /// Evict unpinned LRU residents; skip all eager warm-up.
    Critical = 2,
}

impl GpuMemoryPressure {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Constrained => "constrained",
            Self::Critical => "critical",
        }
    }

    #[cfg(test)]
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Constrained,
            2 => Self::Critical,
            _ => Self::Normal,
        }
    }
}

/// Allocator pressure defaults used when the room resident cap is 2 (Low memory mode).
const LOW_MEMORY_CONSTRAINED_ALLOC_MIB: u64 = 2200;
const LOW_MEMORY_CRITICAL_ALLOC_MIB: u64 = 2800;
/// Integrated-GPU low-memory defaults — shared-memory parts OOM far below the discrete floor.
const INTEGRATED_LOW_MEMORY_CONSTRAINED_ALLOC_MIB: u64 = 384;
const INTEGRATED_LOW_MEMORY_CRITICAL_ALLOC_MIB: u64 = 640;
/// Allocator pressure defaults used when the room resident cap is 6 (Performance/Visuals).
///
/// High-VRAM adapters should not enter "constrained"/"critical" at ~2-3 GiB usage.
const HIGH_CAP_CONSTRAINED_ALLOC_MIB: u64 = 6144;
const HIGH_CAP_CRITICAL_ALLOC_MIB: u64 = 8192;
const CONSTRAINED_ALLOC_ENV: &str = "MAHJURO_GPU_MEM_CONSTRAINED_MIB";
const CRITICAL_ALLOC_ENV: &str = "MAHJURO_GPU_MEM_CRITICAL_MIB";

static LAST_LOGGED: AtomicU8 = AtomicU8::new(0);
static LAST_EAGER_PRELOAD_LOG: AtomicU32 = AtomicU32::new(u32::MAX);
static CONSTRAINED_ALLOC_OVERRIDE_MIB: OnceLock<Option<u64>> = OnceLock::new();
static CRITICAL_ALLOC_OVERRIDE_MIB: OnceLock<Option<u64>> = OnceLock::new();

pub struct PressureSnapshot {
    pub pressure: GpuMemoryPressure,
    pub allocated_bytes: Option<u64>,
    pub reserved_bytes: Option<u64>,
    pub room_gpu_residents: usize,
    pub max_room_gpu_residents: usize,
}

/// Pressure tier for optional eager warm-up — relaxed on Performance/Visuals presets.
pub fn eager_warm_pressure(
    snapshot: &PressureSnapshot,
    mode: mahjuro_gfx_types::GraphicsMode,
) -> GpuMemoryPressure {
    if mode == mahjuro_gfx_types::GraphicsMode::LowMemory {
        return snapshot.pressure;
    }
    // Cap 6 holds all hub/run rooms; only block when over cap (eviction cannot free a slot).
    if snapshot.room_gpu_residents > snapshot.max_room_gpu_residents {
        GpuMemoryPressure::Critical
    } else {
        GpuMemoryPressure::Normal
    }
}

/// Classify pressure from allocator report (when present) and resident count.
pub fn classify(
    device: &wgpu::Device,
    room_gpu_residents: usize,
    max_room_gpu_residents: usize,
    integrated_gpu: bool,
) -> PressureSnapshot {
    let report = device.generate_allocator_report();
    let allocated_bytes = report.as_ref().map(|r| r.total_allocated_bytes);
    let reserved_bytes = report.as_ref().map(|r| r.total_reserved_bytes);

    let (constrained_alloc_mib, critical_alloc_mib) =
        allocator_pressure_thresholds_mib(max_room_gpu_residents, integrated_gpu);
    let pressure = if let Some(allocated) = allocated_bytes {
        let mib = allocated / (1024 * 1024);
        if mib >= critical_alloc_mib {
            GpuMemoryPressure::Critical
        } else if mib >= constrained_alloc_mib {
            GpuMemoryPressure::Constrained
        } else {
            resident_fallback(room_gpu_residents, max_room_gpu_residents)
        }
    } else {
        resident_fallback(room_gpu_residents, max_room_gpu_residents)
    };

    PressureSnapshot {
        pressure,
        allocated_bytes,
        reserved_bytes,
        room_gpu_residents,
        max_room_gpu_residents,
    }
}

fn allocator_pressure_thresholds_mib(
    max_room_gpu_residents: usize,
    integrated_gpu: bool,
) -> (u64, u64) {
    let (mut constrained, mut critical) =
        default_allocator_pressure_thresholds_mib(max_room_gpu_residents, integrated_gpu);
    if let Some(v) =
        *CONSTRAINED_ALLOC_OVERRIDE_MIB.get_or_init(|| env_override_mib(CONSTRAINED_ALLOC_ENV))
    {
        constrained = v;
    }
    if let Some(v) =
        *CRITICAL_ALLOC_OVERRIDE_MIB.get_or_init(|| env_override_mib(CRITICAL_ALLOC_ENV))
    {
        critical = v;
    }
    ordered_allocator_thresholds_mib(constrained, critical)
}

fn default_allocator_pressure_thresholds_mib(
    max_room_gpu_residents: usize,
    integrated_gpu: bool,
) -> (u64, u64) {
    // Low-memory preset currently caps room residents at 2.
    if max_room_gpu_residents <= 2 {
        if integrated_gpu {
            return (
                INTEGRATED_LOW_MEMORY_CONSTRAINED_ALLOC_MIB,
                INTEGRATED_LOW_MEMORY_CRITICAL_ALLOC_MIB,
            );
        }
        (
            LOW_MEMORY_CONSTRAINED_ALLOC_MIB,
            LOW_MEMORY_CRITICAL_ALLOC_MIB,
        )
    } else {
        (HIGH_CAP_CONSTRAINED_ALLOC_MIB, HIGH_CAP_CRITICAL_ALLOC_MIB)
    }
}

fn ordered_allocator_thresholds_mib(constrained: u64, critical: u64) -> (u64, u64) {
    if critical <= constrained {
        (constrained, constrained.saturating_add(1))
    } else {
        (constrained, critical)
    }
}

fn env_override_mib(name: &str) -> Option<u64> {
    let raw = std::env::var(name).ok()?;
    let parsed = raw.trim().parse::<u64>().ok()?;
    if parsed == 0 {
        return None;
    }
    Some(parsed)
}

fn resident_fallback(residents: usize, cap: usize) -> GpuMemoryPressure {
    if residents > cap {
        GpuMemoryPressure::Critical
    } else if residents >= cap {
        GpuMemoryPressure::Constrained
    } else {
        GpuMemoryPressure::Normal
    }
}

/// Log when the pressure tier changes (always at `info` when profiling env is set).
pub fn log_pressure_transition(snapshot: &PressureSnapshot) {
    let tag = snapshot.pressure as u8;
    if LAST_LOGGED.swap(tag, Ordering::Relaxed) == tag {
        return;
    }
    if !crate::gpu_memory_profile::enabled() {
        return;
    }
    match (snapshot.allocated_bytes, snapshot.reserved_bytes) {
        (Some(alloc), Some(res)) => log::info!(
            "gpu mem profile: pressure={} residents={}/{} allocated={} MiB reserved={} MiB",
            snapshot.pressure.label(),
            snapshot.room_gpu_residents,
            snapshot.max_room_gpu_residents,
            alloc / (1024 * 1024),
            res / (1024 * 1024),
        ),
        _ => log::info!(
            "gpu mem profile: pressure={} residents={}/{} (allocator report unavailable)",
            snapshot.pressure.label(),
            snapshot.room_gpu_residents,
            snapshot.max_room_gpu_residents,
        ),
    }
}

/// Log an eager preload decision when profiling is enabled.
pub fn log_eager_preload(action: &'static str, room: &'static str, pressure: GpuMemoryPressure) {
    if !crate::gpu_memory_profile::enabled() {
        return;
    }
    // Paused lines repeat every frame under critical pressure — log once per (action, room, tier).
    let key = (action.as_bytes()[0] as u32)
        .wrapping_mul(31)
        .wrapping_add(room.as_bytes().get(0).copied().unwrap_or(0) as u32)
        .wrapping_mul(31)
        .wrapping_add(pressure as u32);
    if action == "paused" && LAST_EAGER_PRELOAD_LOG.swap(key, Ordering::Relaxed) == key {
        return;
    }
    if action == "paused" {
        LAST_EAGER_PRELOAD_LOG.store(key, Ordering::Relaxed);
    }
    log::info!(
        "gpu mem profile: eager preload {action} {room} (pressure={pressure})",
        action = action,
        room = room,
        pressure = pressure.label(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_fallback_tiers() {
        assert_eq!(resident_fallback(0, 6), GpuMemoryPressure::Normal);
        assert_eq!(resident_fallback(6, 6), GpuMemoryPressure::Constrained);
        assert_eq!(resident_fallback(7, 6), GpuMemoryPressure::Critical);
    }

    #[test]
    fn eager_warm_pressure_relaxed_off_low_memory() {
        let snap = PressureSnapshot {
            pressure: GpuMemoryPressure::Constrained,
            allocated_bytes: None,
            reserved_bytes: None,
            room_gpu_residents: 6,
            max_room_gpu_residents: 6,
        };
        assert_eq!(
            eager_warm_pressure(&snap, mahjuro_gfx_types::GraphicsMode::Visuals),
            GpuMemoryPressure::Normal,
        );
        assert_eq!(
            eager_warm_pressure(&snap, mahjuro_gfx_types::GraphicsMode::LowMemory),
            GpuMemoryPressure::Constrained,
        );
    }

    #[test]
    fn pressure_labels() {
        assert_eq!(GpuMemoryPressure::Normal.label(), "normal");
        assert_eq!(GpuMemoryPressure::from_u8(2), GpuMemoryPressure::Critical);
    }

    #[test]
    fn default_allocator_thresholds_follow_resident_cap() {
        assert_eq!(
            default_allocator_pressure_thresholds_mib(2, false),
            (
                LOW_MEMORY_CONSTRAINED_ALLOC_MIB,
                LOW_MEMORY_CRITICAL_ALLOC_MIB
            )
        );
        assert_eq!(
            default_allocator_pressure_thresholds_mib(2, true),
            (
                INTEGRATED_LOW_MEMORY_CONSTRAINED_ALLOC_MIB,
                INTEGRATED_LOW_MEMORY_CRITICAL_ALLOC_MIB
            )
        );
        assert_eq!(
            default_allocator_pressure_thresholds_mib(6, false),
            (HIGH_CAP_CONSTRAINED_ALLOC_MIB, HIGH_CAP_CRITICAL_ALLOC_MIB)
        );
    }

    #[test]
    fn allocator_thresholds_keep_critical_above_constrained() {
        assert_eq!(ordered_allocator_thresholds_mib(2200, 2800), (2200, 2800));
        assert_eq!(ordered_allocator_thresholds_mib(3000, 2000), (3000, 3001));
    }
}
