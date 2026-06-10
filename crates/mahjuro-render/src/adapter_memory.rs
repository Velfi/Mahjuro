//! Query OS-reported adapter VRAM via wgpu HAL for graphics preset selection.

use mahjuro_gfx_types::AdapterMemoryProbe;
use wgpu_hal::AdapterMemoryCaps;

fn caps_to_probe(caps: AdapterMemoryCaps) -> AdapterMemoryProbe {
    AdapterMemoryProbe {
        dedicated_vram_bytes: caps.dedicated_bytes,
        device_local_heap_bytes: caps.device_local_heap_bytes,
        shared_system_bytes: caps.shared_system_bytes,
        has_unified_memory: caps.has_unified_memory,
    }
}

/// Best-effort adapter memory inspection before `request_device`.
pub fn probe_adapter_memory(adapter: &wgpu::Adapter) -> AdapterMemoryProbe {
    match adapter.get_info().backend {
        #[cfg(windows)]
        wgpu::Backend::Dx12 => probe_dx12(adapter),
        #[cfg(all(not(target_arch = "wasm32"), any(target_os = "linux", target_os = "android", target_os = "windows")))]
        wgpu::Backend::Vulkan => probe_vulkan(adapter),
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        wgpu::Backend::Metal => probe_metal(adapter),
        _ => AdapterMemoryProbe::default(),
    }
}

#[cfg(windows)]
fn probe_dx12(adapter: &wgpu::Adapter) -> AdapterMemoryProbe {
    use wgpu_hal::api::Dx12;
    // SAFETY: HAL adapter outlives this call; read-only OS query.
    let caps = unsafe {
        adapter
            .as_hal::<Dx12>()
            .map(|hal| hal.memory_caps())
            .unwrap_or_default()
    };
    caps_to_probe(caps)
}

#[cfg(all(not(target_arch = "wasm32"), any(target_os = "linux", target_os = "android", target_os = "windows")))]
fn probe_vulkan(adapter: &wgpu::Adapter) -> AdapterMemoryProbe {
    use wgpu_hal::api::Vulkan;
    let caps = unsafe {
        adapter
            .as_hal::<Vulkan>()
            .map(|hal| hal.memory_caps())
            .unwrap_or_default()
    };
    caps_to_probe(caps)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn probe_metal(adapter: &wgpu::Adapter) -> AdapterMemoryProbe {
    use wgpu_hal::api::Metal;
    let caps = unsafe {
        adapter
            .as_hal::<Metal>()
            .map(|hal| hal.memory_caps())
            .unwrap_or_default()
    };
    caps_to_probe(caps)
}

pub fn log_adapter_memory_probe(probe: &AdapterMemoryProbe) {
    let dedicated = probe
        .effective_discrete_vram_mib()
        .map(|m| format!("{m} MiB"))
        .unwrap_or_else(|| "unknown".into());
    let shared = probe
        .shared_system_mib()
        .map(|m| format!("{m} MiB"))
        .unwrap_or_else(|| "unknown".into());
    let unified = probe
        .has_unified_memory
        .map(|u| u.to_string())
        .unwrap_or_else(|| "unknown".into());
    log::info!(
        "adapter memory probe: dedicated_vram={dedicated} shared_system={shared} unified_memory={unified}"
    );
}
