use serde::{Deserialize, Serialize};

use crate::ShadowQuality;

/// Unified graphics preset: trades GPU cost for shadows, reflections, and VRAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsMode {
    /// Shadows and SSR off; full internal resolution.
    Performance,
    /// Certified path for ~4 GB discrete GPUs at 1080p: no dynamic shadows/SSR/HDR swapchain.
    LowMemory,
    #[default]
    Visuals,
}

pub const MIN_RENDER_WIDTH: u32 = 1280;
pub const MIN_RENDER_HEIGHT: u32 = 720;
/// Product support floor for graphics memory budgeting.
pub const MIN_SUPPORTED_GPU_MEMORY_MIB: u64 = 4096;
/// Discrete adapters reporting less than this many MiB of VRAM default to Low memory.
pub const AUTO_LOW_MEMORY_VRAM_THRESHOLD_MIB: u64 = MIN_SUPPORTED_GPU_MEMORY_MIB + 1024;

/// OS-reported adapter memory at startup (capacity, not current process usage).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdapterMemoryProbe {
    pub dedicated_vram_bytes: Option<u64>,
    pub device_local_heap_bytes: Option<u64>,
    pub shared_system_bytes: Option<u64>,
    pub has_unified_memory: Option<bool>,
}

impl AdapterMemoryProbe {
    /// Non-zero dedicated VRAM, else device-local heap total.
    pub fn effective_discrete_vram_bytes(self) -> Option<u64> {
        self.dedicated_vram_bytes
            .filter(|&b| b > 0)
            .or(self.device_local_heap_bytes.filter(|&b| b > 0))
    }

    pub fn effective_discrete_vram_mib(self) -> Option<u64> {
        self.effective_discrete_vram_bytes()
            .map(|b| b / (1024 * 1024))
    }

    pub fn shared_system_mib(self) -> Option<u64> {
        self.shared_system_bytes
            .map(|b| b / (1024 * 1024))
    }

    /// True when OS-reported discrete VRAM is below [`AUTO_LOW_MEMORY_VRAM_THRESHOLD_MIB`].
    pub fn suggests_low_memory_preset(self, integrated_gpu: bool) -> bool {
        if integrated_gpu || self.has_unified_memory == Some(true) {
            return false;
        }
        self.effective_discrete_vram_mib()
            .is_some_and(|mib| mib < AUTO_LOW_MEMORY_VRAM_THRESHOLD_MIB)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BandwidthClass {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsMemoryModel {
    DedicatedVram {
        dedicated_vram_mb: Option<u64>,
    },
    UnifiedMemory {
        system_memory_mb: Option<u64>,
        bandwidth_class: Option<BandwidthClass>,
    },
    Unknown,
}

impl GraphicsMemoryModel {
    pub fn classify_adapter(name: &str, integrated_gpu: bool) -> Self {
        Self::classify_adapter_with_memory(name, integrated_gpu, None)
    }

    pub fn classify_adapter_with_memory(
        name: &str,
        integrated_gpu: bool,
        memory: Option<&AdapterMemoryProbe>,
    ) -> Self {
        if integrated_gpu
            || adapter_name_suggests_apple_silicon(name)
            || adapter_name_suggests_steam_deck(name)
            || memory.is_some_and(|m| m.has_unified_memory == Some(true))
        {
            let bandwidth_class = if adapter_name_suggests_apple_silicon(name)
                || adapter_name_suggests_steam_deck(name)
            {
                Some(BandwidthClass::High)
            } else if memory.is_some_and(|m| {
                m.effective_discrete_vram_mib()
                    .is_some_and(|mib| mib < AUTO_LOW_MEMORY_VRAM_THRESHOLD_MIB)
            }) {
                Some(BandwidthClass::Low)
            } else {
                Some(BandwidthClass::Medium)
            };
            return Self::UnifiedMemory {
                system_memory_mb: memory.and_then(|m| m.shared_system_mib()),
                bandwidth_class,
            };
        }
        Self::DedicatedVram {
            dedicated_vram_mb: memory.and_then(|m| m.effective_discrete_vram_mib()),
        }
    }
}

impl GraphicsMode {
    pub fn next(self) -> Self {
        match self {
            Self::Performance => Self::LowMemory,
            Self::LowMemory => Self::Visuals,
            Self::Visuals => Self::Performance,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Performance => Self::Visuals,
            Self::LowMemory => Self::Performance,
            Self::Visuals => Self::LowMemory,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Performance => "Performance",
            Self::LowMemory => "Low memory",
            Self::Visuals => "Visuals",
        }
    }

    #[inline]
    pub fn shadow_quality(self) -> ShadowQuality {
        match self {
            Self::Performance | Self::LowMemory => ShadowQuality::Off,
            Self::Visuals => ShadowQuality::High,
        }
    }

    #[inline]
    pub fn ssr_enabled(self) -> bool {
        matches!(self, Self::Visuals)
    }

    /// HDR swapchain output is disabled on the low-memory preset to save WSI bytes.
    #[inline]
    pub fn hdr_swapchain_enabled(self) -> bool {
        !matches!(self, Self::LowMemory)
    }

    /// Fraction of the window size used for scene/depth/bloom intermediates (always native).
    #[inline]
    pub fn render_scale(self) -> f32 {
        1.0
    }

    /// Maximum room GLB environments kept resident on the GPU at once.
    #[inline]
    pub fn max_room_gpu_residents(self) -> usize {
        match self {
            Self::LowMemory => 2,
            Self::Performance | Self::Visuals => 6,
        }
    }

    /// Maximum inactive showcase decal atlases cached on the GPU.
    #[inline]
    pub fn max_showcase_decal_atlas_cache(self) -> usize {
        match self {
            Self::LowMemory => 1,
            Self::Performance | Self::Visuals => 4,
        }
    }

    /// Parse `MAHJURO_GRAPHICS_MODE` when set (`performance` / `low_memory` / `visuals`).
    pub fn from_env_override() -> Option<Self> {
        let raw = std::env::var_os("MAHJURO_GRAPHICS_MODE")?;
        let s = raw.to_string_lossy();
        match s.as_ref() {
            "performance" => Some(Self::Performance),
            "low_memory" | "low-memory" | "lowmemory" => Some(Self::LowMemory),
            "visuals" => Some(Self::Visuals),
            _ => None,
        }
    }

    /// Heuristic default when the player has not chosen a preset.
    pub fn suggest_for_adapter(name: &str, integrated_gpu: bool) -> Self {
        Self::suggest_for_adapter_with_memory(name, integrated_gpu, None)
    }

    /// Like [`Self::suggest_for_adapter`] but may use OS-reported adapter VRAM when available.
    pub fn suggest_for_adapter_with_memory(
        name: &str,
        integrated_gpu: bool,
        memory: Option<&AdapterMemoryProbe>,
    ) -> Self {
        if let Some(mode) = Self::from_env_override() {
            return mode;
        }
        if std::env::var_os("MAHJURO_AUTO_LOW_MEMORY").is_some() {
            return Self::LowMemory;
        }
        if memory.is_some_and(|m| m.suggests_low_memory_preset(integrated_gpu)) {
            return Self::LowMemory;
        }
        let model = GraphicsMemoryModel::classify_adapter_with_memory(name, integrated_gpu, memory);
        if matches!(model, GraphicsMemoryModel::UnifiedMemory { .. }) {
            if integrated_gpu
                && !adapter_name_suggests_apple_silicon(name)
                && !adapter_name_suggests_steam_deck(name)
                && memory.is_none()
            {
                return Self::LowMemory;
            }
            return Self::Visuals;
        }
        if memory.is_none() {
            return Self::LowMemory;
        }
        Self::Visuals
    }

    /// Best-effort adapter heuristic for the minimum supported graphics-memory floor.
    ///
    /// Returns `false` when the adapter is likely below the 4 GiB support target.
    pub fn adapter_meets_minimum_support(name: &str, integrated_gpu: bool) -> bool {
        Self::adapter_meets_minimum_support_with_memory(name, integrated_gpu, None)
    }

    pub fn adapter_meets_minimum_support_with_memory(
        name: &str,
        integrated_gpu: bool,
        memory: Option<&AdapterMemoryProbe>,
    ) -> bool {
        if let Some(mib) = memory.and_then(|m| m.effective_discrete_vram_mib()) {
            return mib >= MIN_SUPPORTED_GPU_MEMORY_MIB;
        }
        let model = GraphicsMemoryModel::classify_adapter_with_memory(name, integrated_gpu, memory);
        match model {
            GraphicsMemoryModel::UnifiedMemory { .. } => {
                adapter_name_suggests_apple_silicon(name)
                    || adapter_name_suggests_steam_deck(name)
            }
            GraphicsMemoryModel::DedicatedVram { .. } => false,
            GraphicsMemoryModel::Unknown => false,
        }
    }

    /// Infer a preset from legacy per-field settings (pre-unification saves).
    pub fn from_legacy(shadow_quality: ShadowQuality, ssr_enabled: bool) -> Self {
        if shadow_quality.active() || ssr_enabled {
            Self::Visuals
        } else {
            Self::Performance
        }
    }
}

/// Apple M-series / SoC GPUs — unified memory, not low-VRAM discrete targets.
fn adapter_name_suggests_apple_silicon(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("apple m") || n.contains("apple a") || n.contains("apple gpu")
}

/// Steam Deck (Van Gogh / custom AMD APU) adapters on Vulkan/Proton stacks.
fn adapter_name_suggests_steam_deck(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("vangogh")
        || n.contains("steam deck")
        || n.contains("amd custom gpu 0405")
        || n.contains("custom gpu 0405")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_disables_heavy_passes() {
        let m = GraphicsMode::Performance;
        assert!(!m.shadow_quality().active());
        assert!(!m.ssr_enabled());
        assert_eq!(m.render_scale(), 1.0);
    }

    #[test]
    fn low_memory_tightens_budget() {
        let m = GraphicsMode::LowMemory;
        assert!(!m.shadow_quality().active());
        assert!(!m.ssr_enabled());
        assert!(!m.hdr_swapchain_enabled());
        assert_eq!(m.render_scale(), 1.0);
        assert_eq!(m.max_room_gpu_residents(), 2);
    }

    #[test]
    fn visuals_enables_heavy_passes() {
        let m = GraphicsMode::Visuals;
        assert_eq!(m.shadow_quality(), ShadowQuality::High);
        assert!(m.ssr_enabled());
    }

    #[test]
    fn legacy_migration() {
        assert_eq!(
            GraphicsMode::from_legacy(ShadowQuality::Off, false),
            GraphicsMode::Performance,
        );
        assert_eq!(
            GraphicsMode::from_legacy(ShadowQuality::High, false),
            GraphicsMode::Visuals,
        );
        assert_eq!(
            GraphicsMode::from_legacy(ShadowQuality::Off, true),
            GraphicsMode::Visuals,
        );
    }

    #[test]
    fn probed_4gb_discrete_defaults_to_low_memory() {
        let probe = AdapterMemoryProbe {
            dedicated_vram_bytes: Some(4 * 1024 * 1024 * 1024),
            device_local_heap_bytes: Some(4 * 1024 * 1024 * 1024),
            shared_system_bytes: None,
            has_unified_memory: Some(false),
        };
        assert!(probe.suggests_low_memory_preset(false));
        assert_eq!(
            GraphicsMode::suggest_for_adapter_with_memory(
                "NVIDIA GeForce RTX 3060",
                false,
                Some(&probe),
            ),
            GraphicsMode::LowMemory,
        );
    }

    #[test]
    fn probed_8gb_discrete_defaults_to_visuals() {
        let probe = AdapterMemoryProbe {
            dedicated_vram_bytes: Some(8 * 1024 * 1024 * 1024),
            device_local_heap_bytes: Some(8 * 1024 * 1024 * 1024),
            shared_system_bytes: None,
            has_unified_memory: Some(false),
        };
        assert!(!probe.suggests_low_memory_preset(false));
        assert_eq!(
            GraphicsMode::suggest_for_adapter_with_memory(
                "NVIDIA GeForce RTX 4070",
                false,
                Some(&probe),
            ),
            GraphicsMode::Visuals,
        );
    }

    #[test]
    fn memory_model_records_probed_vram() {
        let probe = AdapterMemoryProbe {
            dedicated_vram_bytes: Some(8 * 1024 * 1024 * 1024),
            ..Default::default()
        };
        assert!(matches!(
            GraphicsMemoryModel::classify_adapter_with_memory(
                "NVIDIA GeForce RTX 4070",
                false,
                Some(&probe),
            ),
            GraphicsMemoryModel::DedicatedVram {
                dedicated_vram_mb: Some(8192),
            }
        ));
    }

    #[test]
    fn apple_silicon_not_low_vram() {
        assert!(adapter_name_suggests_apple_silicon("Apple M4 Max"));
        assert_eq!(
            GraphicsMode::suggest_for_adapter("Apple M4 Max", true),
            GraphicsMode::Visuals,
        );
    }

    #[test]
    fn minimum_support_heuristic() {
        assert_eq!(MIN_SUPPORTED_GPU_MEMORY_MIB, 4096);
        let mx250 = AdapterMemoryProbe {
            dedicated_vram_bytes: Some(2 * 1024 * 1024 * 1024),
            ..Default::default()
        };
        assert!(!GraphicsMode::adapter_meets_minimum_support_with_memory(
            "NVIDIA GeForce MX250",
            false,
            Some(&mx250),
        ));
        let intel = AdapterMemoryProbe {
            device_local_heap_bytes: Some(2 * 1024 * 1024 * 1024),
            has_unified_memory: Some(true),
            ..Default::default()
        };
        assert!(!GraphicsMode::adapter_meets_minimum_support_with_memory(
            "Intel Iris Xe",
            true,
            Some(&intel),
        ));
        assert!(GraphicsMode::adapter_meets_minimum_support(
            "Apple M4 Max",
            true
        ));
        let rx7900 = AdapterMemoryProbe {
            dedicated_vram_bytes: Some(24 * 1024 * 1024 * 1024),
            ..Default::default()
        };
        assert!(GraphicsMode::adapter_meets_minimum_support_with_memory(
            "AMD Radeon RX 7900 XT",
            false,
            Some(&rx7900),
        ));
        assert!(GraphicsMode::adapter_meets_minimum_support(
            "AMD Custom GPU 0405 (RADV VANGOGH)",
            true
        ));
    }

    #[test]
    fn steam_deck_defaults_to_visuals_not_low_memory() {
        assert_eq!(
            GraphicsMode::suggest_for_adapter("AMD Custom GPU 0405 (RADV VANGOGH)", true),
            GraphicsMode::Visuals
        );
    }

    #[test]
    fn memory_model_classifies_unified_platforms() {
        assert!(matches!(
            GraphicsMemoryModel::classify_adapter("AMD Custom GPU 0405 (RADV VANGOGH)", true),
            GraphicsMemoryModel::UnifiedMemory { .. }
        ));
        assert!(matches!(
            GraphicsMemoryModel::classify_adapter("Apple M4 Max", true),
            GraphicsMemoryModel::UnifiedMemory { .. }
        ));
        assert!(matches!(
            GraphicsMemoryModel::classify_adapter("NVIDIA GeForce RTX 4070", false),
            GraphicsMemoryModel::DedicatedVram { .. }
        ));
    }
}
