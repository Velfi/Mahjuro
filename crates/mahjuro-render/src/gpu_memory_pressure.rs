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

/// Fallback when OS VRAM probe is unavailable (4 GiB support target).
const FALLBACK_DISCRETE_VRAM_MIB: u64 = mahjuro_gfx_types::MIN_SUPPORTED_GPU_MEMORY_MIB;
/// wgpu allocator totals omit swapchain images, pipeline caches, and driver reserve.
const UNTRACKED_GPU_OVERHEAD_MIB: u64 = 512;
/// Low-memory preset: enter constrained / critical at this fraction of probed VRAM budget.
const LOW_MEMORY_CONSTRAINED_VRAM_PCT: u64 = 55;
const LOW_MEMORY_CRITICAL_VRAM_PCT: u64 = 70;
/// High-cap preset defaults when VRAM probe is unavailable.
const HIGH_CAP_CONSTRAINED_ALLOC_MIB: u64 = 6144;
const HIGH_CAP_CRITICAL_ALLOC_MIB: u64 = 8192;
/// Integrated-GPU low-memory defaults — shared-memory parts OOM far below the discrete floor.
const INTEGRATED_LOW_MEMORY_CONSTRAINED_ALLOC_MIB: u64 = 384;
const INTEGRATED_LOW_MEMORY_CRITICAL_ALLOC_MIB: u64 = 640;
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
    /// DXGI / VK_EXT_memory_budget usage when available (includes untracked bytes).
    pub os_usage_bytes: Option<u64>,
    pub os_budget_bytes: Option<u64>,
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

fn effective_usage_bytes(allocated_bytes: Option<u64>, reserved_bytes: Option<u64>) -> Option<u64> {
    match (allocated_bytes, reserved_bytes) {
        (Some(a), Some(r)) => Some(a.max(r)),
        (Some(a), None) => Some(a),
        (None, Some(r)) => Some(r),
        (None, None) => None,
    }
}

/// Classify pressure from allocator report (when present) and resident count.
pub fn classify(
    device: &wgpu::Device,
    room_gpu_residents: usize,
    max_room_gpu_residents: usize,
    integrated_gpu: bool,
    adapter_memory: &mahjuro_gfx_types::AdapterMemoryProbe,
) -> PressureSnapshot {
    let report = device.generate_allocator_report();
    let allocated_bytes = report.as_ref().map(|r| r.total_allocated_bytes);
    let reserved_bytes = report.as_ref().map(|r| r.total_reserved_bytes);
    let os_usage = crate::adapter_memory::probe_device_memory_usage(device);
    let os_usage_bytes = os_usage.map(|u| u.current_bytes);
    let os_budget_bytes = os_usage.map(|u| u.budget_bytes);

    let (constrained_alloc_mib, critical_alloc_mib) = allocator_pressure_thresholds_mib(
        max_room_gpu_residents,
        integrated_gpu,
        adapter_memory,
        os_budget_bytes,
    );
    let pressure = if let Some(usage) =
        effective_pressure_bytes(allocated_bytes, reserved_bytes, os_usage_bytes)
    {
        let mib = usage / (1024 * 1024);
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
        os_usage_bytes,
        os_budget_bytes,
        room_gpu_residents,
        max_room_gpu_residents,
    }
}

fn effective_pressure_bytes(
    allocated_bytes: Option<u64>,
    reserved_bytes: Option<u64>,
    os_usage_bytes: Option<u64>,
) -> Option<u64> {
    let allocator = effective_usage_bytes(allocated_bytes, reserved_bytes);
    match (allocator, os_usage_bytes) {
        (Some(a), Some(o)) => Some(a.max(o)),
        (Some(a), None) => Some(a),
        (None, Some(o)) => Some(o),
        (None, None) => None,
    }
}

fn probed_discrete_vram_mib(memory: &mahjuro_gfx_types::AdapterMemoryProbe) -> u64 {
    memory
        .effective_discrete_vram_mib()
        .unwrap_or(FALLBACK_DISCRETE_VRAM_MIB)
}

fn vram_fraction_thresholds_mib(
    vram_mib: u64,
    constrained_pct: u64,
    critical_pct: u64,
    subtract_untracked_overhead: bool,
) -> (u64, u64) {
    let budget = if subtract_untracked_overhead {
        vram_mib.saturating_sub(UNTRACKED_GPU_OVERHEAD_MIB).max(256)
    } else {
        vram_mib.max(256)
    };
    let constrained = budget.saturating_mul(constrained_pct) / 100;
    let critical = budget.saturating_mul(critical_pct) / 100;
    ordered_allocator_thresholds_mib(
        constrained.max(256),
        critical.max(constrained.saturating_add(256)),
    )
}

fn allocator_pressure_thresholds_mib(
    max_room_gpu_residents: usize,
    integrated_gpu: bool,
    adapter_memory: &mahjuro_gfx_types::AdapterMemoryProbe,
    os_budget_bytes: Option<u64>,
) -> (u64, u64) {
    let (mut constrained, mut critical) = default_allocator_pressure_thresholds_mib(
        max_room_gpu_residents,
        integrated_gpu,
        adapter_memory,
        os_budget_bytes,
    );
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
    adapter_memory: &mahjuro_gfx_types::AdapterMemoryProbe,
    os_budget_bytes: Option<u64>,
) -> (u64, u64) {
    let os_budget_mib = os_budget_bytes.map(|b| b / (1024 * 1024));
    let subtract_overhead = os_budget_mib.is_none();
    let budget_mib = os_budget_mib.unwrap_or_else(|| probed_discrete_vram_mib(adapter_memory));
    // Low-memory preset currently caps room residents at 2.
    if max_room_gpu_residents <= 2 {
        if integrated_gpu {
            return (
                INTEGRATED_LOW_MEMORY_CONSTRAINED_ALLOC_MIB,
                INTEGRATED_LOW_MEMORY_CRITICAL_ALLOC_MIB,
            );
        }
        return vram_fraction_thresholds_mib(
            budget_mib,
            LOW_MEMORY_CONSTRAINED_VRAM_PCT,
            LOW_MEMORY_CRITICAL_VRAM_PCT,
            subtract_overhead,
        );
    }
    if budget_mib <= HIGH_CAP_CRITICAL_ALLOC_MIB {
        vram_fraction_thresholds_mib(budget_mib, 75, 90, subtract_overhead)
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
    match (
        snapshot.allocated_bytes,
        snapshot.reserved_bytes,
        snapshot.os_usage_bytes,
    ) {
        (Some(alloc), Some(res), Some(os)) => log::info!(
            "gpu mem profile: pressure={} residents={}/{} allocated={} MiB reserved={} MiB os_usage={} MiB os_budget={} MiB effective_usage={} MiB",
            snapshot.pressure.label(),
            snapshot.room_gpu_residents,
            snapshot.max_room_gpu_residents,
            alloc / (1024 * 1024),
            res / (1024 * 1024),
            os / (1024 * 1024),
            snapshot.os_budget_bytes.unwrap_or(0) / (1024 * 1024),
            effective_pressure_bytes(Some(alloc), Some(res), Some(os)).unwrap_or(0) / (1024 * 1024),
        ),
        (Some(alloc), Some(res), None) => log::info!(
            "gpu mem profile: pressure={} residents={}/{} allocated={} MiB reserved={} MiB usage={} MiB",
            snapshot.pressure.label(),
            snapshot.room_gpu_residents,
            snapshot.max_room_gpu_residents,
            alloc / (1024 * 1024),
            res / (1024 * 1024),
            effective_usage_bytes(Some(alloc), Some(res)).unwrap_or(0) / (1024 * 1024),
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
            os_usage_bytes: None,
            os_budget_bytes: None,
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
    fn usage_bytes_prefers_os_when_higher() {
        assert_eq!(
            effective_pressure_bytes(Some(100), Some(250), Some(3000)),
            Some(3000)
        );
        assert_eq!(
            effective_pressure_bytes(Some(300), Some(250), Some(200)),
            Some(300)
        );
    }

    #[test]
    fn usage_bytes_prefers_reserved_when_higher() {
        assert_eq!(effective_usage_bytes(Some(100), Some(250)), Some(250));
        assert_eq!(effective_usage_bytes(Some(300), Some(250)), Some(300));
    }

    #[test]
    fn low_memory_thresholds_scale_with_probed_vram() {
        let probe_4g = mahjuro_gfx_types::AdapterMemoryProbe {
            dedicated_vram_bytes: Some(4 * 1024 * 1024 * 1024),
            ..Default::default()
        };
        let (c, k) = default_allocator_pressure_thresholds_mib(2, false, &probe_4g, None);
        // (4096 - 512) * 55% = 1971, * 70% = 2508
        assert_eq!(c, 1971);
        assert_eq!(k, 2508);

        let (c_os, k_os) = default_allocator_pressure_thresholds_mib(
            2,
            false,
            &probe_4g,
            Some(3 * 1024 * 1024 * 1024),
        );
        // 3072 * 55% = 1689, * 70% = 2150 — OS budget below dedicated capacity.
        assert_eq!(c_os, 1689);
        assert_eq!(k_os, 2150);

        let probe_3g = mahjuro_gfx_types::AdapterMemoryProbe {
            dedicated_vram_bytes: Some(3 * 1024 * 1024 * 1024),
            ..Default::default()
        };
        let (c3, k3) = default_allocator_pressure_thresholds_mib(2, false, &probe_3g, None);
        assert!(c3 < c);
        assert!(k3 < k);
    }

    #[test]
    fn default_allocator_thresholds_follow_resident_cap() {
        let probe = mahjuro_gfx_types::AdapterMemoryProbe {
            dedicated_vram_bytes: Some(4 * 1024 * 1024 * 1024),
            ..Default::default()
        };
        assert_eq!(
            default_allocator_pressure_thresholds_mib(2, false, &probe, None),
            (1971, 2508)
        );
        assert_eq!(
            default_allocator_pressure_thresholds_mib(2, true, &probe, None),
            (
                INTEGRATED_LOW_MEMORY_CONSTRAINED_ALLOC_MIB,
                INTEGRATED_LOW_MEMORY_CRITICAL_ALLOC_MIB
            )
        );
        assert_eq!(
            default_allocator_pressure_thresholds_mib(6, false, &probe, None),
            (2688, 3225)
        );
    }

    #[test]
    fn allocator_thresholds_keep_critical_above_constrained() {
        assert_eq!(ordered_allocator_thresholds_mib(2200, 2800), (2200, 2800));
        assert_eq!(ordered_allocator_thresholds_mib(3000, 2000), (3000, 3001));
    }
}
