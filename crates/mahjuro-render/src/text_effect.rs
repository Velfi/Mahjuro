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
    /// Score-pop polychrome bands — same albedo look as extruded cascade glyphs.
    Polychrome = 5,
    /// Moonlight bands — `#e8ebf0` stripes on a `#3a4565` field (**The Moon**).
    MoonPolychrome = 6,
}

impl TextEffectId {
    /// Fragment presets that read `globals.time` (see `shaders/text_quad.wgsl`).
    pub const fn uses_time_in_fragment(self) -> bool {
        matches!(
            self,
            Self::Rainbow | Self::Pulse | Self::Shimmer | Self::Polychrome | Self::MoonPolychrome
        )
    }

    /// Stripped to [`Flat`] when [`mahjuro_gfx_types::EffectsQuality`] is Off/Low
    /// (see `flatten_time_text_fx` in the UI text draw path). [`Polychrome`] and
    /// [`MoonPolychrome`] are exempt — cheap sin bands, and glossary proper nouns
    /// should match score pops / moonlight headlines.
    pub const fn flattened_when_effects_low(self) -> bool {
        matches!(self, Self::Rainbow | Self::Pulse | Self::Shimmer)
    }

    pub const fn pack(self) -> u32 {
        self as u32
    }

    /// Pack fragment effect (low byte) and clockwise quarter-turns (bits 8–9).
    pub fn pack_with_rotation(self, rotation_quarters: u8) -> u32 {
        self.pack() | ((rotation_quarters.min(3) as u32) << 8)
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

#[cfg(test)]
mod tests {
    use super::TextEffectId;

    #[test]
    fn polychrome_survives_low_effects_gate() {
        assert!(!TextEffectId::Polychrome.flattened_when_effects_low());
        assert!(!TextEffectId::MoonPolychrome.flattened_when_effects_low());
        assert!(TextEffectId::Rainbow.flattened_when_effects_low());
    }
}
