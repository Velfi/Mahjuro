use serde::{Deserialize, Serialize};

use crate::ShadowQuality;

/// Unified graphics preset: trades GPU cost for shadows and table reflections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsMode {
    Performance,
    #[default]
    Visuals,
}

impl GraphicsMode {
    pub fn next(self) -> Self {
        match self {
            Self::Performance => Self::Visuals,
            Self::Visuals => Self::Performance,
        }
    }

    pub fn prev(self) -> Self {
        self.next()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Performance => "Performance",
            Self::Visuals => "Visuals",
        }
    }

    #[inline]
    pub fn shadow_quality(self) -> ShadowQuality {
        match self {
            Self::Performance => ShadowQuality::Off,
            Self::Visuals => ShadowQuality::High,
        }
    }

    #[inline]
    pub fn ssr_enabled(self) -> bool {
        matches!(self, Self::Visuals)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_disables_heavy_passes() {
        let m = GraphicsMode::Performance;
        assert!(!m.shadow_quality().active());
        assert!(!m.ssr_enabled());
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
}
