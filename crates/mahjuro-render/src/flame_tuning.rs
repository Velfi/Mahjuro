//! Shop / gameplay candle flame tuning — shader shell + emitter placement.

pub const FLAME_TUNING_SCENE_KEY: &str = "shop_gameplay_candles";

/// Live-editable candle flame parameters (shader + scene placement).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FlameTuning {
    /// Godot base mesh height multiplier (`flame.wgsl` `FLAME_HEIGHT` numerator).
    pub mesh_height_base: f32,
    /// Godot mesh height scale (`flame.wgsl` `FLAME_HEIGHT` multiplier).
    pub mesh_height_scale: f32,
    /// Revolved shell width multiplier (`flame.wgsl` `FLAME_WIDTH`).
    pub mesh_width_mul: f32,
    /// World scale on candle height for flame emitter size.
    pub emitter_scale_mul: f32,
    pub taper_factor: f32,
    pub height_rw_rate: f32,
    pub height_rw_amp: f32,
    pub bright_rw_rate: f32,
    pub micro_rw_rate: f32,
    pub bright_rw_amp: f32,
    pub emission_gain: f32,
    pub max_alpha: f32,
    pub border_width: f32,
    pub bottom_fade_y_start: f32,
    pub bottom_fade_y_end: f32,
    /// Shop / gameplay `light_candle*` punctual + procedural flame flicker swing.
    pub candle_flicker_amp: f32,
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
            mesh_height_base: 1.72,
            mesh_height_scale: 4.0,
            mesh_width_mul: 1.64,
            emitter_scale_mul: 0.22,
            taper_factor: 0.88,
            height_rw_rate: 17.0,
            height_rw_amp: 0.016,
            bright_rw_rate: 15.0,
            micro_rw_rate: 19.0,
            bright_rw_amp: 0.018,
            emission_gain: 10.5,
            max_alpha: 0.72,
            border_width: 0.75,
            bottom_fade_y_start: 0.0,
            bottom_fade_y_end: 0.14,
            candle_flicker_amp: 0.019,
            lightbake_height_frac: 0.48,
            wick_below_light_frac: 0.42,
        }
    }

    #[inline]
    pub fn mesh_height(&self) -> f32 {
        self.mesh_height_base * self.mesh_height_scale
    }

    #[inline]
    pub fn emitter_scale(&self, candle_world_scale: f32) -> f32 {
        candle_world_scale * self.emitter_scale_mul
    }

    #[inline]
    pub fn wick_from_light(&self, light_world: glam::Vec3, emitter_scale: f32) -> glam::Vec3 {
        light_world
            - glam::Vec3::new(0.0, 0.0, emitter_scale * self.wick_below_light_frac)
    }

    #[inline]
    pub fn flame_height_world(&self, room_world_scale: f32, candle_doc_height_m: f32) -> f32 {
        candle_doc_height_m
            * room_world_scale.max(1e-6)
            * self.lightbake_height_frac
            * self.mesh_height_scale
    }

    /// GPU uniform block appended to [`super::wgpu_renderer::uniforms::FlameViewUniform`].
    pub fn shader_fields(&self) -> [f32; 13] {
        [
            self.mesh_height(),
            self.mesh_width_mul,
            self.taper_factor,
            self.height_rw_rate,
            self.height_rw_amp,
            self.bright_rw_rate,
            self.micro_rw_rate,
            self.bright_rw_amp,
            self.emission_gain,
            self.max_alpha,
            self.border_width,
            self.bottom_fade_y_start,
            self.bottom_fade_y_end,
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
            0 => self.mesh_height_base,
            1 => self.mesh_height_scale,
            2 => self.mesh_width_mul,
            3 => self.emitter_scale_mul,
            4 => self.taper_factor,
            5 => self.height_rw_rate,
            6 => self.height_rw_amp,
            7 => self.bright_rw_rate,
            8 => self.micro_rw_rate,
            9 => self.bright_rw_amp,
            10 => self.emission_gain,
            11 => self.max_alpha,
            12 => self.border_width,
            13 => self.bottom_fade_y_start,
            14 => self.bottom_fade_y_end,
            15 => self.candle_flicker_amp,
            16 => self.lightbake_height_frac,
            17 => self.wick_below_light_frac,
            _ => 0.0,
        }
    }

    pub fn set_debug_row_value(&mut self, row: usize, v: f32) {
        let (_, lo, hi, _) = FLAME_DEBUG_ROW_META[row];
        let v = v.clamp(lo, hi);
        match row {
            0 => self.mesh_height_base = v,
            1 => self.mesh_height_scale = v,
            2 => self.mesh_width_mul = v,
            3 => self.emitter_scale_mul = v,
            4 => self.taper_factor = v,
            5 => self.height_rw_rate = v,
            6 => self.height_rw_amp = v,
            7 => self.bright_rw_rate = v,
            8 => self.micro_rw_rate = v,
            9 => self.bright_rw_amp = v,
            10 => self.emission_gain = v,
            11 => self.max_alpha = v,
            12 => self.border_width = v,
            13 => self.bottom_fade_y_start = v,
            14 => self.bottom_fade_y_end = v,
            15 => self.candle_flicker_amp = v,
            16 => self.lightbake_height_frac = v,
            17 => self.wick_below_light_frac = v,
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
                "    mesh_height_base: {:.4},\n",
                "    mesh_height_scale: {:.4},\n",
                "    mesh_width_mul: {:.4},\n",
                "    emitter_scale_mul: {:.4},\n",
                "    taper_factor: {:.4},\n",
                "    height_rw_rate: {:.4},\n",
                "    height_rw_amp: {:.4},\n",
                "    bright_rw_rate: {:.4},\n",
                "    micro_rw_rate: {:.4},\n",
                "    bright_rw_amp: {:.4},\n",
                "    emission_gain: {:.4},\n",
                "    max_alpha: {:.4},\n",
                "    border_width: {:.4},\n",
                "    bottom_fade_y_start: {:.4},\n",
                "    bottom_fade_y_end: {:.4},\n",
                "    candle_flicker_amp: {:.4},\n",
                "    lightbake_height_frac: {:.4},\n",
                "    wick_below_light_frac: {:.4},\n",
                "}};\n",
            ),
            self.mesh_height_base,
            self.mesh_height_scale,
            self.mesh_width_mul,
            self.emitter_scale_mul,
            self.taper_factor,
            self.height_rw_rate,
            self.height_rw_amp,
            self.bright_rw_rate,
            self.micro_rw_rate,
            self.bright_rw_amp,
            self.emission_gain,
            self.max_alpha,
            self.border_width,
            self.bottom_fade_y_start,
            self.bottom_fade_y_end,
            self.candle_flicker_amp,
            self.lightbake_height_frac,
            self.wick_below_light_frac,
        )
    }
}

pub const FLAME_DEBUG_ROW_META: &[(&str, f32, f32, f32)] = &[
    ("Mesh height base", 0.5, 4.0, 0.02),
    ("Mesh height scale", 0.5, 8.0, 0.05),
    ("Mesh width mul", 0.2, 4.0, 0.02),
    ("Emitter scale mul", 0.05, 0.8, 0.005),
    ("Taper factor", 0.0, 1.0, 0.01),
    ("Height RW rate (Hz)", 1.0, 40.0, 0.5),
    ("Height RW amp", 0.0, 0.08, 0.001),
    ("Bright RW rate (Hz)", 1.0, 40.0, 0.5),
    ("Micro RW rate (Hz)", 1.0, 40.0, 0.5),
    ("Bright RW amp", 0.0, 0.08, 0.001),
    ("Emission gain", 0.0, 30.0, 0.1),
    ("Max alpha", 0.1, 1.0, 0.01),
    ("Border width", 0.0, 1.0, 0.01),
    ("Bottom fade y start", 0.0, 0.5, 0.01),
    ("Bottom fade y end", 0.0, 0.5, 0.01),
    ("Candle flicker amp", 0.0, 0.08, 0.001),
    ("Lightbake height frac", 0.1, 1.2, 0.01),
    ("Wick below light frac", 0.0, 1.0, 0.01),
];

pub const FLAME_DEBUG_SLIDER_COUNT: usize = FLAME_DEBUG_ROW_META.len();
