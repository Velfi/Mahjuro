//! Packed `TextEffectId` for screen-space text quads (`GpuInstance.user`).
//! Fragment shaders branch on the low byte; high bits reserved for future
//! speed/phase presets.

/// Whitelist of fragment-driven text looks. Unknown markup maps to [`Flat`].
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextEffectId {
    #[default]
    Flat = 0,
    Rainbow = 1,
    Pulse = 2,
    Shimmer = 3,
    /// Warm metallic tint modulation (shader preset, not a PBR texture).
    GoldTint = 4,
}

impl TextEffectId {
    /// Fragment presets that read `globals.time` (see `shaders/text_quad.wgsl`).
    pub const fn uses_time_in_fragment(self) -> bool {
        matches!(self, Self::Rainbow | Self::Pulse | Self::Shimmer)
    }

    pub const fn pack(self) -> u32 {
        self as u32
    }

    /// Pack fragment effect (low byte) and clockwise quarter-turns (bits 8–9).
    pub fn pack_with_rotation(self, rotation_quarters: u8) -> u32 {
        self.pack() | ((rotation_quarters.min(3) as u32) << 8)
    }

    #[allow(dead_code)] // Reserved for CPU-side inspection / future tooling.
    pub fn decode(packed: u32) -> Self {
        match (packed & 0xff) as u8 {
            1 => Self::Rainbow,
            2 => Self::Pulse,
            3 => Self::Shimmer,
            4 => Self::GoldTint,
            _ => Self::Flat,
        }
    }

    /// Resolve `effect:name` from safe markup (ASCII lower snake).
    pub fn from_markup_name(name: &str) -> Option<Self> {
        match name.trim() {
            "flat" | "" => Some(Self::Flat),
            "rainbow" => Some(Self::Rainbow),
            "pulse" => Some(Self::Pulse),
            "shimmer" => Some(Self::Shimmer),
            "gold" | "gold_tint" => Some(Self::GoldTint),
            _ => None,
        }
    }
}
