use serde::{Deserialize, Serialize};

/// Shadow map resolution and sampling tier. Every GPU-buffer light casts a
/// projected depth map when quality > Off; tiers scale map size only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum ShadowQuality {
    Off,
    Low,
    #[default]
    Medium,
    High,
}

impl ShadowQuality {
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

    #[inline]
    pub fn active(self) -> bool {
        !matches!(self, Self::Off)
    }

    #[inline]
    pub fn point_map_size(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::Low => 256,
            Self::Medium | Self::High => 512,
        }
    }

    #[inline]
    pub fn spot_map_size(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::Low | Self::Medium => 512,
            Self::High => 1024,
        }
    }

    #[inline]
    pub fn particle_shadows(self) -> bool {
        matches!(self, Self::Medium | Self::High)
    }

    #[inline]
    pub fn contact_ao(self) -> bool {
        self.active()
    }
}
