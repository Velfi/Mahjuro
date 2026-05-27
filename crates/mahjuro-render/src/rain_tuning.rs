//! Main-menu rain: CPU world-space field only ([`crate::rain_field::RainField`]).

pub const RAIN_TUNING_SCENE_KEY: &str = "main_menu_exterior";

/// CPU rain-field simulation (world drops + splashes on `rain_hit_*` shells).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RainFieldTuning {
    /// Multiplier on [`Self::spawn_rate`] and [`Self::pool_size`] (0 = none, 1 = nominal).
    #[serde(default = "default_field_density")]
    pub density: f32,
    pub spawn_rate: f32,
    pub pool_size: f32,
    /// World −Z speed (units/s) before [`RainTuning::speed_mul`].
    pub fall_speed: f32,
    pub wind_x: f32,
    pub wind_y: f32,
    /// Target on-screen streak length in **pixels** (full span along fall direction).
    pub streak_len_px: f32,
    pub splash_count: f32,
    pub splash_lifetime: f32,
    pub drop_color: [f32; 4],
    /// XY padding around the room AABB as a fraction of each axis extent.
    #[serde(default = "default_field_volume_pad_xy")]
    pub volume_pad_xy: f32,
    /// Spawn ceiling: room top + this × room Z extent.
    #[serde(default = "default_field_volume_top_mul")]
    pub volume_top_mul: f32,
    /// View-depth spawn falloff toward the camera (0 = uniform, ~2.2 = shipped default).
    #[serde(default = "default_field_spawn_near_bias")]
    pub spawn_near_bias: f32,
}

fn default_field_density() -> f32 {
    1.0
}

fn default_field_volume_pad_xy() -> f32 {
    0.5
}

fn default_field_volume_top_mul() -> f32 {
    0.65
}

fn default_field_spawn_near_bias() -> f32 {
    2.2
}

/// World rain tuning — [`Self::field`] plus global [`Self::speed_mul`].
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RainTuning {
    /// Scales field fall, wind, and spawn rate. Persists for saved overlays / headless.
    pub speed_mul: f32,
    pub field: RainFieldTuning,
}

impl Default for RainTuning {
    fn default() -> Self {
        Self::shipping_default()
    }
}

impl RainTuning {
    pub fn shipping_default() -> Self {
        Self {
            speed_mul: 3.1504,
            field: RainFieldTuning {
                density: 0.7963,
                spawn_rate: 280.0,
                pool_size: 226.6241,
                fall_speed: 8000.0,
                wind_x: 423.818,
                wind_y: 800.0,
                streak_len_px: 48.0,
                splash_count: 15.9531,
                splash_lifetime: 0.3331,
                drop_color: [0.75, 0.82, 0.95, 0.62],
                volume_pad_xy: 0.5045,
                volume_top_mul: 0.65,
                spawn_near_bias: 2.9529,
            },
        }
    }

    /// World −Z fall speed for 3D drops (units/s).
    pub fn field_fall_speed_world(&self) -> f32 {
        self.field.fall_speed.max(0.0) * self.speed_mul.max(0.0)
    }

    pub fn storage_key() -> String {
        format!("RainTuning:{}", RAIN_TUNING_SCENE_KEY)
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
            0 => self.speed_mul,
            1 => self.field.density,
            2 => self.field.spawn_rate,
            3 => self.field.pool_size,
            4 => self.field.fall_speed,
            5 => self.field.wind_x,
            6 => self.field.wind_y,
            7 => self.field.streak_len_px,
            8 => self.field.splash_count,
            9 => self.field.splash_lifetime,
            10 => rgb_linear_hue([
                self.field.drop_color[0],
                self.field.drop_color[1],
                self.field.drop_color[2],
            ]),
            11 => rgb_linear_sat([
                self.field.drop_color[0],
                self.field.drop_color[1],
                self.field.drop_color[2],
            ]),
            12 => self.field.drop_color[3],
            13 => self.field.volume_pad_xy,
            14 => self.field.volume_top_mul,
            15 => self.field.spawn_near_bias,
            _ => 0.0,
        }
    }

    pub fn set_debug_row_value(&mut self, row: usize, v: f32) {
        let (_, lo, hi, _) = RAIN_DEBUG_ROW_META[row];
        let v = v.clamp(lo, hi);
        match row {
            0 => self.speed_mul = v,
            1 => self.field.density = v,
            2 => self.field.spawn_rate = v,
            3 => self.field.pool_size = v,
            4 => self.field.fall_speed = v,
            5 => self.field.wind_x = v,
            6 => self.field.wind_y = v,
            7 => self.field.streak_len_px = v,
            8 => self.field.splash_count = v,
            9 => self.field.splash_lifetime = v,
            10 => {
                let mut rgb = [
                    self.field.drop_color[0],
                    self.field.drop_color[1],
                    self.field.drop_color[2],
                ];
                set_linear_rgb_hue(&mut rgb, v);
                self.field.drop_color[0..3].copy_from_slice(&rgb);
            }
            11 => {
                let mut rgb = [
                    self.field.drop_color[0],
                    self.field.drop_color[1],
                    self.field.drop_color[2],
                ];
                set_linear_rgb_sat(&mut rgb, v);
                self.field.drop_color[0..3].copy_from_slice(&rgb);
            }
            12 => self.field.drop_color[3] = v,
            13 => self.field.volume_pad_xy = v,
            14 => self.field.volume_top_mul = v,
            15 => self.field.spawn_near_bias = v,
            _ => {}
        }
    }

    /// Rust literal for pasting into code (Enter / Ctrl+C in the rain debug overlay).
    pub fn to_rust_literal(self) -> String {
        format!(
            concat!(
                "// RainTuning snapshot — assign to `renderer.rain_tuning` or replace ",
                "`RainTuning::shipping_default()` in `rain_tuning.rs`\n",
                "use crate::rain_tuning::{{RainFieldTuning, RainTuning}};\n",
                "const RAIN_TUNING: RainTuning = RainTuning {{\n",
                "    speed_mul: {:.4},\n",
                "    field: RainFieldTuning {{\n",
                "        density: {:.4}, spawn_rate: {:.4}, pool_size: {:.4}, fall_speed: {:.4},\n",
                "        wind_x: {:.4}, wind_y: {:.4}, streak_len_px: {:.4}, splash_count: {:.4},\n",
                "        splash_lifetime: {:.4},\n",
                "        drop_color: [{:.4}, {:.4}, {:.4}, {:.4}],\n",
                "        volume_pad_xy: {:.4}, volume_top_mul: {:.4},\n",
                "        spawn_near_bias: {:.4},\n",
                "    }},\n",
                "}};\n",
            ),
            self.speed_mul,
            self.field.density,
            self.field.spawn_rate,
            self.field.pool_size,
            self.field.fall_speed,
            self.field.wind_x,
            self.field.wind_y,
            self.field.streak_len_px,
            self.field.splash_count,
            self.field.splash_lifetime,
            self.field.drop_color[0],
            self.field.drop_color[1],
            self.field.drop_color[2],
            self.field.drop_color[3],
            self.field.volume_pad_xy,
            self.field.volume_top_mul,
            self.field.spawn_near_bias,
        )
    }
}

fn rgb_linear_to_hsv(rgb: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (rgb[0], rgb[1], rgb[2]);
    let v = r.max(g).max(b);
    let min = r.min(g).min(b);
    if v <= 1e-8 {
        return (0.0, 0.0, 0.0);
    }
    let d = v - min;
    if d <= 1e-8 {
        return (0.0, 0.0, v);
    }
    let s = d / v;
    let h = if (v - r).abs() < 1e-8 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (v - g).abs() < 1e-8 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h.fract(), s, v)
}

fn hsv_to_rgb_linear(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.fract() * 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (r, g, b) = match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [r, g, b]
}

#[inline]
fn rgb_linear_hue(rgb: [f32; 3]) -> f32 {
    rgb_linear_to_hsv(rgb).0
}

#[inline]
fn rgb_linear_sat(rgb: [f32; 3]) -> f32 {
    rgb_linear_to_hsv(rgb).1
}

fn set_linear_rgb_hue(rgb: &mut [f32; 3], hue: f32) {
    let (_, s, v) = rgb_linear_to_hsv(*rgb);
    *rgb = hsv_to_rgb_linear(hue, s, v);
}

fn set_linear_rgb_sat(rgb: &mut [f32; 3], sat: f32) {
    let (h, _, v) = rgb_linear_to_hsv(*rgb);
    *rgb = hsv_to_rgb_linear(h, sat, v);
}

/// Peak-normalized RGB for color swatches in the rain debug overlay.
pub fn rain_color_swatch_rgb(tuning: &RainTuning, row: usize) -> Option<[f32; 3]> {
    let rgb = match row {
        10..=11 => [
            tuning.field.drop_color[0],
            tuning.field.drop_color[1],
            tuning.field.drop_color[2],
        ],
        _ => return None,
    };
    let peak = rgb[0].max(rgb[1]).max(rgb[2]);
    if peak <= 1e-8 {
        return Some([0.0; 3]);
    }
    Some([rgb[0] / peak, rgb[1] / peak, rgb[2] / peak])
}

pub fn rain_row_is_hue(row: usize) -> bool {
    row == 10
}

pub fn rain_row_is_saturation(row: usize) -> bool {
    row == 11
}

/// Full-saturation linear RGB at hue `h` (hue-wheel slider art).
pub fn rain_hue_wheel_preview_linear(hue: f32) -> [f32; 3] {
    hsv_to_rgb_linear(hue.fract(), 1.0, 1.0)
}

pub const RAIN_DEBUG_ROW_META: &[(&str, f32, f32, f32)] = &[
    ("Speed × (fall, wind, spawn)", 0.0, 30.0, 0.1),
    ("Density (× spawn rate & pool)", 0.0, 25.0, 0.25),
    ("Spawn rate (drops/s)", 0.0, 800.0, 5.0),
    ("Pool size", 0.0, 600.0, 10.0),
    ("Fall speed (world Z u/s)", 0.0, 8000.0, 20.0),
    ("Wind X", -800.0, 800.0, 5.0),
    ("Wind Y", -800.0, 800.0, 5.0),
    ("Streak length (px)", 2.0, 512.0, 1.0),
    ("Splash count", 1.0, 24.0, 1.0),
    ("Splash lifetime", 0.05, 1.5, 0.02),
    ("Drop hue", 0.0, 1.0, 1.0 / 360.0),
    ("Drop saturation", 0.0, 1.0, 0.01),
    ("Drop alpha", 0.0, 1.0, 0.01),
    ("Volume pad XY (fraction)", 0.0, 2.0, 0.02),
    ("Volume top (× room Z)", 0.1, 2.0, 0.02),
    ("Spawn near-camera bias", 0.0, 6.0, 0.05),
];

/// Slider rows in the rain debug overlay (Save / Reset / Close follow).
pub const RAIN_DEBUG_SLIDER_COUNT: usize = RAIN_DEBUG_ROW_META.len();
