use serde::{Deserialize, Serialize};

use crate::ShadowQuality;

/// Unified graphics preset: trades GPU cost for shadows, reflections, and VRAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsMode {
    /// Shadows and SSR off; full internal resolution.
    Performance,
    /// Certified path for ~4 GB discrete GPUs at 1080p: reduced internal resolution,
    /// no dynamic shadows/SSR/HDR swapchain.
    LowMemory,
    #[default]
    Visuals,
}

/// Internal 3D render resolution as a fraction of the window size.
pub const LOW_MEMORY_RENDER_SCALE: f32 = 0.75;
pub const MIN_RENDER_WIDTH: u32 = 1280;
pub const MIN_RENDER_HEIGHT: u32 = 720;

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

    /// Fraction of the window size used for scene/depth/bloom intermediates (1.0 = native).
    #[inline]
    pub fn render_scale(self) -> f32 {
        match self {
            Self::LowMemory => LOW_MEMORY_RENDER_SCALE,
            Self::Performance | Self::Visuals => 1.0,
        }
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
        if let Some(mode) = Self::from_env_override() {
            return mode;
        }
        if std::env::var_os("MAHJURO_AUTO_LOW_MEMORY").is_some() {
            return Self::LowMemory;
        }
        if adapter_name_suggests_low_vram(name) || integrated_gpu {
            return Self::LowMemory;
        }
        Self::Visuals
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

/// Substrings that commonly appear on 4 GB (or smaller) discrete GPUs.
fn adapter_name_suggests_low_vram(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "gtx 1050",
        "gtx 1630",
        "gtx 1650",
        "mx150",
        "mx250",
        "mx330",
        "mx350",
        "mx450",
        "mx550",
        "rx 550",
        "rx 560",
        "rx 6400",
        "arc a380",
        "radeon 550",
        "radeon 560",
        "quadro p620",
        "quadro p1000",
    ];
    NEEDLES.iter().any(|needle| n.contains(needle))
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
        assert_eq!(m.render_scale(), LOW_MEMORY_RENDER_SCALE);
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
    fn adapter_name_heuristic() {
        assert!(adapter_name_suggests_low_vram("NVIDIA GeForce GTX 1050 Ti"));
        assert!(!adapter_name_suggests_low_vram("NVIDIA GeForce RTX 4070"));
    }
}
