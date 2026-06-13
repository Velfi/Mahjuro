//! Shop / gameplay candle flame tuning — digital-garden plume + palette layers.

pub const FLAME_TUNING_SCENE_KEY: &str = "shop_gameplay_candles";

/// Live-editable candle flame parameters (shader + scene placement).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FlameTuning {
    /// Height multiplier on the unit plume mesh (world height ≈ this × emitter scale).
    pub flame_height_mul: f32,
    /// Whole-column wind lean (0 = upright indoor candle).
    pub wind_strength: f32,
    /// FBM turbulence strength on the plume surface.
    pub turbulence: f32,
    /// RGB multiplier on all flame layers.
    pub emission_gain: f32,
    /// Radial envelope scale (legacy `mesh_width_mul`).
    pub flame_width_mul: f32,
    /// World scale on candle height for flame emitter size.
    pub emitter_scale_mul: f32,
    /// Shop / gameplay `light_candle*` punctual + procedural flame flicker swing.
    pub candle_flicker_amp: f32,
    /// Steady wind lean bias in world X (added to each flame instance).
    pub wind_bias_x: f32,
    /// Steady wind lean bias in world Y (added to each flame instance).
    pub wind_bias_y: f32,
    /// Lightbake height as fraction of shop candle world height.
    pub lightbake_height_frac: f32,
    /// Nudge flame anchor below `light_candle*` empties (fraction of emitter scale).
    pub wick_below_light_frac: f32,
}

impl Default for FlameTuning {
    fn default() -> Self {
        Self::shipping_default()
    }
}

impl FlameTuning {
    pub fn shipping_default() -> Self {
        Self {
            // +10 mm tip height at reference shop candle scale (0.052 m × 0.22 emitter mul).
            flame_height_mul: 4.87,
            wind_strength: 0.45,
            turbulence: 0.75,
            emission_gain: 1.0,
            flame_width_mul: 1.64,
            emitter_scale_mul: 0.22,
            candle_flicker_amp: 0.03,
            wind_bias_x: 0.0,
            wind_bias_y: 0.0,
            lightbake_height_frac: 0.48,
            wick_below_light_frac: 0.42,
        }
    }

    #[inline]
    pub fn emitter_scale(&self, candle_world_scale: f32) -> f32 {
        candle_world_scale * self.emitter_scale_mul
    }

    #[inline]
    pub fn wick_from_light(&self, light_world: glam::Vec3, emitter_scale: f32) -> glam::Vec3 {
        light_world - glam::Vec3::new(0.0, 0.0, emitter_scale * self.wick_below_light_frac)
    }

    #[inline]
    pub fn flame_height_world(&self, room_world_scale: f32, candle_doc_height_m: f32) -> f32 {
        candle_doc_height_m
            * room_world_scale.max(1e-6)
            * self.lightbake_height_frac
            * self.flame_height_mul
    }

    /// GPU uniform block appended to [`super::wgpu_renderer::uniforms::FlameViewUniform`].
    pub fn shader_fields(&self) -> [f32; 8] {
        [
            self.flame_height_mul,
            self.wind_strength,
            self.turbulence,
            self.emission_gain,
            self.flame_width_mul,
            self.candle_flicker_amp,
            self.wind_bias_x,
            self.wind_bias_y,
        ]
    }

    pub fn storage_key() -> String {
        format!("FlameTuning:{}", FLAME_TUNING_SCENE_KEY)
    }

    pub fn load() -> Self {
        if mahjuro_gfx_types::has_tuning_override(&Self::storage_key()) {
            mahjuro_gfx_types::load_tuning_override(&Self::storage_key())
        } else {
            Self::shipping_default()
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        mahjuro_gfx_types::save_tuning_override(&Self::storage_key(), self)
    }

    pub fn clear_saved() -> anyhow::Result<()> {
        mahjuro_gfx_types::clear_tuning_override(&Self::storage_key())
    }

    pub fn debug_row_value(self, row: usize) -> f32 {
        match row {
            0 => self.flame_height_mul,
            1 => self.wind_strength,
            2 => self.turbulence,
            3 => self.emission_gain,
            4 => self.flame_width_mul,
            5 => self.emitter_scale_mul,
            6 => self.candle_flicker_amp,
            7 => self.wind_bias_x,
            8 => self.wind_bias_y,
            9 => self.lightbake_height_frac,
            10 => self.wick_below_light_frac,
            _ => 0.0,
        }
    }

    pub fn set_debug_row_value(&mut self, row: usize, v: f32) {
        let (_, lo, hi, _) = FLAME_DEBUG_ROW_META[row];
        let v = v.clamp(lo, hi);
        match row {
            0 => self.flame_height_mul = v,
            1 => self.wind_strength = v,
            2 => self.turbulence = v,
            3 => self.emission_gain = v,
            4 => self.flame_width_mul = v,
            5 => self.emitter_scale_mul = v,
            6 => self.candle_flicker_amp = v,
            7 => self.wind_bias_x = v,
            8 => self.wind_bias_y = v,
            9 => self.lightbake_height_frac = v,
            10 => self.wick_below_light_frac = v,
            _ => {}
        }
    }

    pub fn to_rust_literal(self) -> String {
        format!(
            concat!(
                "// FlameTuning snapshot — assign to `renderer.flame_tuning` or replace ",
                "`FlameTuning::shipping_default()` in `flame_tuning.rs`\n",
                "use crate::flame_tuning::FlameTuning;\n",
                "const FLAME_TUNING: FlameTuning = FlameTuning {{\n",
                "    flame_height_mul: {:.4},\n",
                "    wind_strength: {:.4},\n",
                "    turbulence: {:.4},\n",
                "    emission_gain: {:.4},\n",
                "    flame_width_mul: {:.4},\n",
                "    emitter_scale_mul: {:.4},\n",
                "    candle_flicker_amp: {:.4},\n",
                "    wind_bias_x: {:.4},\n",
                "    wind_bias_y: {:.4},\n",
                "    lightbake_height_frac: {:.4},\n",
                "    wick_below_light_frac: {:.4},\n",
                "}};\n",
            ),
            self.flame_height_mul,
            self.wind_strength,
            self.turbulence,
            self.emission_gain,
            self.flame_width_mul,
            self.emitter_scale_mul,
            self.candle_flicker_amp,
            self.wind_bias_x,
            self.wind_bias_y,
            self.lightbake_height_frac,
            self.wick_below_light_frac,
        )
    }
}

pub const FLAME_DEBUG_ROW_META: &[(&str, f32, f32, f32)] = &[
    ("Plume · height mul", 0.5, 10.0, 0.05),
    ("Plume · wind strength", 0.0, 1.5, 0.01),
    ("Plume · turbulence", 0.0, 2.0, 0.01),
    ("Shader · emission gain", 0.0, 4.0, 0.05),
    ("Plume · width mul", 0.2, 4.0, 0.02),
    ("Placement · emitter scale", 0.05, 0.8, 0.005),
    ("Light · flicker amp", 0.0, 0.08, 0.001),
    ("Wind · bias X", -1.0, 1.0, 0.05),
    ("Wind · bias Y", -1.0, 1.0, 0.05),
    ("Light · lightbake height", 0.1, 1.2, 0.01),
    ("Placement · wick offset", 0.0, 1.0, 0.01),
];

pub const FLAME_DEBUG_SLIDER_COUNT: usize = FLAME_DEBUG_ROW_META.len();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_rows_cover_all_fields() {
        assert_eq!(FLAME_DEBUG_SLIDER_COUNT, 11);
        let base = FlameTuning::shipping_default();
        for row in 0..FLAME_DEBUG_SLIDER_COUNT {
            let v = base.debug_row_value(row);
            let mut edited = base;
            edited.set_debug_row_value(row, v + FLAME_DEBUG_ROW_META[row].3);
            assert_ne!(edited.debug_row_value(row), v);
        }
    }
}
