//! GPU memory pressure classification for eager room preload gating.
//!
//! Uses wgpu allocator totals when available (Vulkan/DX12); falls back to resident
//! count vs the active [`mahjuro_gfx_types::GraphicsMode`] cap on Metal/GLES.

use std::sync::atomic::{AtomicU8, Ordering};

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

/// Allocator `total_allocated_bytes` at/above which optional eager warm-up pauses.
const CONSTRAINED_ALLOC_MIB: u64 = 2200;
/// Allocator `total_allocated_bytes` at/above which eager work stops and eviction runs.
const CRITICAL_ALLOC_MIB: u64 = 2800;

static LAST_LOGGED: AtomicU8 = AtomicU8::new(0);

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
) -> PressureSnapshot {
    let report = device.generate_allocator_report();
    let allocated_bytes = report.as_ref().map(|r| r.total_allocated_bytes);
    let reserved_bytes = report.as_ref().map(|r| r.total_reserved_bytes);

    let pressure = if let Some(allocated) = allocated_bytes {
        let mib = allocated / (1024 * 1024);
        if mib >= CRITICAL_ALLOC_MIB {
            GpuMemoryPressure::Critical
        } else if mib >= CONSTRAINED_ALLOC_MIB {
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
        assert_eq!(
            resident_fallback(0, 6),
            GpuMemoryPressure::Normal
        );
        assert_eq!(
            resident_fallback(6, 6),
            GpuMemoryPressure::Constrained
        );
        assert_eq!(
            resident_fallback(7, 6),
            GpuMemoryPressure::Critical
        );
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
}
