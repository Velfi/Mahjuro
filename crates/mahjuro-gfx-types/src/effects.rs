use serde::{Deserialize, Serialize};

/// Controls the quality of fullscreen vignette effects (starfield, ember
/// drift, golden dust, shooting-star cascade). Lower levels reduce or skip
/// procedural layers to save GPU ALU on weaker hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EffectsQuality {
    Off,
    Low,
    Medium,
    High,
}

impl EffectsQuality {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Off,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Off => Self::High,
            Self::Low => Self::Off,
            Self::Medium => Self::Low,
            Self::High => Self::Medium,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    /// Numeric quality level uploaded to the GPU globals uniform.
    /// The cascade shader uses this to gate layer groups.
    pub fn quality_level_f32(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::Low => 0.0,
            Self::Medium => 1.0,
            Self::High => 2.0,
        }
    }

    /// Procedural felt shader tier for `lit_mesh.wgsl` (`SsrGlobals.felt.x`).
    /// `0` = minimal baize tint (effects Off), `1` = reduced noise path
    /// (Low), `2` = full detail (Medium/High).
    pub fn felt_shader_lod(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::Low => 1.0,
            Self::Medium | Self::High => 2.0,
        }
    }
}
