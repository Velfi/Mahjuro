//! Optional GPU memory / adapter logging for VRAM budgeting (`MAHJURO_STARTUP_PROFILE=1`
//! or `MAHJURO_GPU_MEM_PROFILE=1`).

use std::sync::OnceLock;
use std::time::Instant;

static ENABLED: OnceLock<bool> = OnceLock::new();

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var_os("MAHJURO_GPU_MEM_PROFILE").is_some()
            || std::env::var_os("MAHJURO_STARTUP_PROFILE").is_some()
    })
}

/// Log adapter identity and optional wgpu allocator totals once at startup.
pub fn log_adapter_startup(adapter: &wgpu::Adapter, device: &wgpu::Device) {
    if !enabled() {
        return;
    }
    let info = adapter.get_info();
    let integrated = info.device_type == wgpu::DeviceType::IntegratedGpu;
    let suggested = mahjuro_gfx_types::GraphicsMode::suggest_for_adapter(&info.name, integrated);
    let meets_minimum =
        mahjuro_gfx_types::GraphicsMode::adapter_meets_minimum_support(&info.name, integrated);
    log::info!(
        "gpu mem profile: adapter='{}' backend={:?} device_type={:?} vendor=0x{:04x} \
         suggested_graphics_mode={:?} minimum_supported_4gib={}",
        info.name,
        info.backend,
        info.device_type,
        info.vendor,
        suggested,
        meets_minimum,
    );
    if let Some(report) = device.generate_allocator_report() {
        log::info!(
            "gpu mem profile: allocator — allocated={} MiB reserved={} MiB ({} live allocs, {} blocks)",
            report.total_allocated_bytes / (1024 * 1024),
            report.total_reserved_bytes / (1024 * 1024),
            report.allocations.len(),
            report.blocks.len(),
        );
    } else {
        log::debug!("gpu mem profile: allocator report unavailable on this backend");
    }
}

/// Log allocator totals after a heavy upload (room GLB, tileset atlas, …).
pub fn log_device_allocator(device: &wgpu::Device, label: &'static str) {
    if !enabled() {
        return;
    }
    let Some(report) = device.generate_allocator_report() else {
        return;
    };
    log::info!(
        "gpu mem profile: {label} — allocated={} MiB reserved={} MiB",
        report.total_allocated_bytes / (1024 * 1024),
        report.total_reserved_bytes / (1024 * 1024),
    );
}

/// Run `f` and log wall time when profiling is enabled.
pub fn measure_upload<T>(label: &'static str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let t0 = Instant::now();
    let out = f();
    log::info!(
        "gpu mem profile: {label} upload — {:.1} ms",
        t0.elapsed().as_secs_f64() * 1000.0
    );
    out
}

fn room_evict_label(bit: u8) -> &'static str {
    crate::room_gpu_resident::RoomGpuResidentId::log_label(bit)
}

/// Log a room GPU eviction (visible at `debug`; allocator follow-up still needs profiling env).
pub fn log_room_evict(bit: u8) {
    log::debug!(
        "room gpu evict: {} (bit 0x{:02x})",
        room_evict_label(bit),
        bit
    );
}
