//! Best-effort adapter VRAM / heap sizing for startup graphics preset selection.

/// OS-reported adapter memory signals (capacity, not current usage).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdapterMemoryCaps {
    /// DXGI `DedicatedVideoMemory` or primary device-local heap total (Vulkan).
    pub dedicated_bytes: Option<u64>,
    /// Sum of Vulkan device-local heap sizes when distinct from [`Self::dedicated_bytes`].
    pub device_local_heap_bytes: Option<u64>,
    /// DXGI `SharedSystemMemory` (integrated / hybrid adapters).
    pub shared_system_bytes: Option<u64>,
    /// Metal `hasUnifiedMemory` when available.
    pub has_unified_memory: Option<bool>,
}

impl AdapterMemoryCaps {
    /// Non-zero dedicated VRAM, else device-local heap total.
    pub fn effective_discrete_bytes(self) -> Option<u64> {
        self.dedicated_bytes
            .filter(|&b| b > 0)
            .or(self.device_local_heap_bytes.filter(|&b| b > 0))
    }
}

/// OS-reported process GPU memory usage vs the current WDDM/Vulkan budget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdapterMemoryUsage {
    pub current_bytes: u64,
    pub budget_bytes: u64,
}
