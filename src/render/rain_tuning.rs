//! Procedural rain vignette + CPU rain-field parameters.

pub const RAIN_TUNING_SCENE_KEY: &str = "main_menu_exterior";

/// GPU upload for `shaders/rain.wgsl` group 1 — must match WGSL `RainParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RainGpuUniform {
    pub layer0: [f32; 4],
    pub layer1: [f32; 4],
    pub layer2: [f32; 4],
    pub scroll: [f32; 4],
    pub bright: [f32; 4],
    pub col0: [f32; 4],
    pub col1: [f32; 4],
    pub col2: [f32; 4],
    pub mist_rgb_strength: [f32; 4],
    pub mist_scale_scroll: [f32; 4],
    pub mist_soft_lower: [f32; 4],
    pub drop_a: [f32; 4],
    pub drop_b: [f32; 4],
    pub drop_c: [f32; 4],
}

/// One depth layer of streaks (near / mid / far).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RainLayerTuning {
    pub scale: f32,
    pub density: f32,
    pub half_len: f32,
    pub half_w: f32,
    pub color: [f32; 3],
    pub brightness: f32,
}

/// Shared per-drop motion / fade knobs inside [`rain_layer`](../../shaders/rain.wgsl).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RainDropTuning {
    pub sway_min: f32,
    pub sway_range: f32,
    pub fall_spd_base: f32,
    pub fall_spd_range: f32,
    pub base_y_min: f32,
    pub base_y_range: f32,
    pub center_x_min: f32,
    pub center_x_range: f32,
    pub fade_in_end: f32,
    pub fade_out_end: f32,
    pub shimmer_base: f32,
    pub shimmer_amp: f32,
}

/// Lower mist band.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RainMistTuning {
    pub color: [f32; 3],
    pub scale: [f32; 2],
    pub scroll: [f32; 2],
    pub soft: [f32; 2],
    pub lower: [f32; 2],
    pub strength: f32,
}

/// CPU rain-field simulation (world drops + splashes on `rain_hit_*` shells).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RainFieldTuning {
    /// > 0.5 enables 3D rain (disables the fullscreen procedural streak pass).
    pub field_enabled: f32,
    pub spawn_rate: f32,
    pub pool_size: f32,
    /// World −Z speed (units/s) before [`RainTuning::speed_mul`].
    pub fall_speed: f32,
    pub wind_x: f32,
    pub wind_y: f32,
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
}

fn default_field_volume_pad_xy() -> f32 {
    0.5
}

fn default_field_volume_top_mul() -> f32 {
    0.65
}

/// Full rain look — defaults match shipped `rain.wgsl` constants.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RainTuning {
    /// Scales all rain motion (streak fall, field scroll, mist). Uploaded as `bright.w`.
    pub speed_mul: f32,
    pub scroll_near: f32,
    pub scroll_mid: f32,
    pub scroll_far: f32,
    pub drop_lean: f32,
    pub near: RainLayerTuning,
    pub mid: RainLayerTuning,
    pub far: RainLayerTuning,
    pub drop: RainDropTuning,
    pub mist: RainMistTuning,
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
            speed_mul: 1.0,
            scroll_near: 0.45,
            scroll_mid: 0.32,
            scroll_far: 0.20,
            drop_lean: 0.06,
            near: RainLayerTuning {
                scale: 22.0,
                density: 0.48,
                half_len: 0.22,
                half_w: 0.014,
                color: [0.82, 0.88, 0.96],
                brightness: 0.55,
            },
            mid: RainLayerTuning {
                scale: 38.0,
                density: 0.52,
                half_len: 0.16,
                half_w: 0.011,
                color: [0.68, 0.76, 0.88],
                brightness: 0.38,
            },
            far: RainLayerTuning {
                scale: 58.0,
                density: 0.56,
                half_len: 0.10,
                half_w: 0.008,
                color: [0.55, 0.64, 0.76],
                brightness: 0.28,
            },
            drop: RainDropTuning {
                sway_min: 0.02,
                sway_range: 0.02,
                fall_spd_base: 0.32,
                fall_spd_range: 0.28,
                base_y_min: 0.15,
                base_y_range: 0.55,
                center_x_min: 0.12,
                center_x_range: 0.76,
                fade_in_end: 0.2,
                fade_out_end: 0.78,
                shimmer_base: 0.82,
                shimmer_amp: 0.18,
            },
            mist: RainMistTuning {
                color: [0.62, 0.70, 0.82],
                scale: [1.15, 0.95],
                scroll: [0.018, 0.009],
                soft: [0.28, 0.72],
                lower: [0.62, 0.08],
                strength: 0.10,
            },
            field: RainFieldTuning {
                field_enabled: 1.0,
                spawn_rate: 280.0,
                pool_size: 520.0,
                fall_speed: 3300.0,
                wind_x: 18.0,
                wind_y: -10.0,
                streak_len_px: 20.0,
                splash_count: 8.0,
                splash_lifetime: 0.18,
                drop_color: [0.75, 0.82, 0.95, 0.62],
                volume_pad_xy: default_field_volume_pad_xy(),
                volume_top_mul: default_field_volume_top_mul(),
            },
        }
    }

    #[inline]
    pub fn field_active(&self) -> bool {
        self.field.field_enabled > 0.5
    }

    /// World −Z fall speed for 3D drops (units/s).
    pub fn field_fall_speed_world(&self) -> f32 {
        self.field.fall_speed.max(0.0) * self.speed_mul.max(0.0)
    }

    pub fn storage_key() -> String {
        format!("RainTuning:{}", RAIN_TUNING_SCENE_KEY)
    }

    pub fn load() -> Self {
        if crate::persistence::has_tuning_override(&Self::storage_key()) {
            crate::persistence::load_tuning_override(&Self::storage_key())
        } else {
            Self::shipping_default()
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        crate::persistence::save_tuning_override(&Self::storage_key(), self)
    }

    pub fn clear_saved() -> anyhow::Result<()> {
        crate::persistence::clear_tuning_override(&Self::storage_key())
    }

    pub fn to_gpu(self) -> RainGpuUniform {
        RainGpuUniform {
            layer0: [
                self.near.scale,
                self.near.density,
                self.near.half_len,
                self.near.half_w,
            ],
            layer1: [
                self.mid.scale,
                self.mid.density,
                self.mid.half_len,
                self.mid.half_w,
            ],
            layer2: [
                self.far.scale,
                self.far.density,
                self.far.half_len,
                self.far.half_w,
            ],
            scroll: [self.scroll_near, self.scroll_mid, self.scroll_far, self.drop_lean],
            bright: [
                self.near.brightness,
                self.mid.brightness,
                self.far.brightness,
                self.speed_mul,
            ],
            col0: [self.near.color[0], self.near.color[1], self.near.color[2], 0.0],
            col1: [self.mid.color[0], self.mid.color[1], self.mid.color[2], 0.0],
            col2: [self.far.color[0], self.far.color[1], self.far.color[2], 0.0],
            mist_rgb_strength: [
                self.mist.color[0],
                self.mist.color[1],
                self.mist.color[2],
                self.mist.strength,
            ],
            mist_scale_scroll: [
                self.mist.scale[0],
                self.mist.scale[1],
                self.mist.scroll[0],
                self.mist.scroll[1],
            ],
            mist_soft_lower: [
                self.mist.soft[0],
                self.mist.soft[1],
                self.mist.lower[0],
                self.mist.lower[1],
            ],
            drop_a: [
                self.drop.sway_min,
                self.drop.sway_range,
                self.drop.fall_spd_base,
                self.drop.fall_spd_range,
            ],
            drop_b: [
                self.drop.base_y_min,
                self.drop.base_y_range,
                self.drop.center_x_min,
                self.drop.center_x_range,
            ],
            drop_c: [
                self.drop.fade_in_end,
                self.drop.fade_out_end,
                self.drop.shimmer_base,
                self.drop.shimmer_amp,
            ],
        }
    }

    pub fn debug_row_value(self, row: usize) -> f32 {
        match row {
            0 => self.speed_mul,
            1 => self.scroll_near,
            2 => self.scroll_mid,
            3 => self.scroll_far,
            4 => self.drop_lean,
            5 => self.near.scale,
            6 => self.near.density,
            7 => self.near.half_len,
            8 => self.near.half_w,
            9 => self.near.brightness,
            10 => rgb_linear_hue(self.near.color),
            11 => rgb_linear_sat(self.near.color),
            12 => self.mid.scale,
            13 => self.mid.density,
            14 => self.mid.half_len,
            15 => self.mid.half_w,
            16 => self.mid.brightness,
            17 => rgb_linear_hue(self.mid.color),
            18 => rgb_linear_sat(self.mid.color),
            19 => self.far.scale,
            20 => self.far.density,
            21 => self.far.half_len,
            22 => self.far.half_w,
            23 => self.far.brightness,
            24 => rgb_linear_hue(self.far.color),
            25 => rgb_linear_sat(self.far.color),
            26 => self.drop.sway_min,
            27 => self.drop.sway_range,
            28 => self.drop.fall_spd_base,
            29 => self.drop.fall_spd_range,
            30 => self.drop.base_y_min,
            31 => self.drop.base_y_range,
            32 => self.drop.center_x_min,
            33 => self.drop.center_x_range,
            34 => self.drop.fade_in_end,
            35 => self.drop.fade_out_end,
            36 => self.drop.shimmer_base,
            37 => self.drop.shimmer_amp,
            38 => rgb_linear_hue(self.mist.color),
            39 => rgb_linear_sat(self.mist.color),
            40 => self.mist.scale[0],
            41 => self.mist.scale[1],
            42 => self.mist.scroll[0],
            43 => self.mist.scroll[1],
            44 => self.mist.soft[0],
            45 => self.mist.soft[1],
            46 => self.mist.lower[0],
            47 => self.mist.lower[1],
            48 => self.mist.strength,
            49 => self.field.field_enabled,
            50 => self.field.spawn_rate,
            51 => self.field.pool_size,
            52 => self.field.fall_speed,
            53 => self.field.wind_x,
            54 => self.field.wind_y,
            55 => self.field.streak_len_px,
            56 => self.field.splash_count,
            57 => self.field.splash_lifetime,
            58 => rgb_linear_hue([
                self.field.drop_color[0],
                self.field.drop_color[1],
                self.field.drop_color[2],
            ]),
            59 => rgb_linear_sat([
                self.field.drop_color[0],
                self.field.drop_color[1],
                self.field.drop_color[2],
            ]),
            60 => self.field.drop_color[3],
            _ => 0.0,
        }
    }

    pub fn set_debug_row_value(&mut self, row: usize, v: f32) {
        let (_, lo, hi, _) = RAIN_DEBUG_ROW_META[row];
        let v = v.clamp(lo, hi);
        match row {
            0 => self.speed_mul = v,
            1 => self.scroll_near = v,
            2 => self.scroll_mid = v,
            3 => self.scroll_far = v,
            4 => self.drop_lean = v,
            5 => self.near.scale = v,
            6 => self.near.density = v,
            7 => self.near.half_len = v,
            8 => self.near.half_w = v,
            9 => self.near.brightness = v,
            10 => set_linear_rgb_hue(&mut self.near.color, v),
            11 => set_linear_rgb_sat(&mut self.near.color, v),
            12 => self.mid.scale = v,
            13 => self.mid.density = v,
            14 => self.mid.half_len = v,
            15 => self.mid.half_w = v,
            16 => self.mid.brightness = v,
            17 => set_linear_rgb_hue(&mut self.mid.color, v),
            18 => set_linear_rgb_sat(&mut self.mid.color, v),
            19 => self.far.scale = v,
            20 => self.far.density = v,
            21 => self.far.half_len = v,
            22 => self.far.half_w = v,
            23 => self.far.brightness = v,
            24 => set_linear_rgb_hue(&mut self.far.color, v),
            25 => set_linear_rgb_sat(&mut self.far.color, v),
            26 => self.drop.sway_min = v,
            27 => self.drop.sway_range = v,
            28 => self.drop.fall_spd_base = v,
            29 => self.drop.fall_spd_range = v,
            30 => self.drop.base_y_min = v,
            31 => self.drop.base_y_range = v,
            32 => self.drop.center_x_min = v,
            33 => self.drop.center_x_range = v,
            34 => self.drop.fade_in_end = v,
            35 => self.drop.fade_out_end = v,
            36 => self.drop.shimmer_base = v,
            37 => self.drop.shimmer_amp = v,
            38 => set_linear_rgb_hue(&mut self.mist.color, v),
            39 => set_linear_rgb_sat(&mut self.mist.color, v),
            40 => self.mist.scale[0] = v,
            41 => self.mist.scale[1] = v,
            42 => self.mist.scroll[0] = v,
            43 => self.mist.scroll[1] = v,
            44 => self.mist.soft[0] = v,
            45 => self.mist.soft[1] = v,
            46 => self.mist.lower[0] = v,
            47 => self.mist.lower[1] = v,
            48 => self.mist.strength = v,
            49 => self.field.field_enabled = v,
            50 => self.field.spawn_rate = v,
            51 => self.field.pool_size = v,
            52 => self.field.fall_speed = v,
            53 => self.field.wind_x = v,
            54 => self.field.wind_y = v,
            55 => self.field.streak_len_px = v,
            56 => self.field.splash_count = v,
            57 => self.field.splash_lifetime = v,
            58 => {
                let mut rgb = [
                    self.field.drop_color[0],
                    self.field.drop_color[1],
                    self.field.drop_color[2],
                ];
                set_linear_rgb_hue(&mut rgb, v);
                self.field.drop_color[0..3].copy_from_slice(&rgb);
            }
            59 => {
                let mut rgb = [
                    self.field.drop_color[0],
                    self.field.drop_color[1],
                    self.field.drop_color[2],
                ];
                set_linear_rgb_sat(&mut rgb, v);
                self.field.drop_color[0..3].copy_from_slice(&rgb);
            }
            60 => self.field.drop_color[3] = v,
            _ => {}
        }
    }

    /// Rust literal for pasting into code (Enter / Ctrl+C in the rain debug overlay).
    pub fn to_rust_literal(self) -> String {
        format!(
            concat!(
                "// RainTuning snapshot — assign to `renderer.rain_tuning` or replace ",
                "`RainTuning::shipping_default()` in `rain_tuning.rs`\n",
                "use crate::render::rain_tuning::{{RainDropTuning, RainLayerTuning, RainMistTuning, RainTuning}};\n",
                "const RAIN_TUNING: RainTuning = RainTuning {{\n",
                "    speed_mul: {:.4},\n",
                "    scroll_near: {:.6},\n",
                "    scroll_mid: {:.6},\n",
                "    scroll_far: {:.6},\n",
                "    drop_lean: {:.6},\n",
                "    near: RainLayerTuning {{\n",
                "        scale: {:.4}, density: {:.4}, half_len: {:.4}, half_w: {:.5},\n",
                "        color: [{:.4}, {:.4}, {:.4}], brightness: {:.4},\n",
                "    }},\n",
                "    mid: RainLayerTuning {{\n",
                "        scale: {:.4}, density: {:.4}, half_len: {:.4}, half_w: {:.5},\n",
                "        color: [{:.4}, {:.4}, {:.4}], brightness: {:.4},\n",
                "    }},\n",
                "    far: RainLayerTuning {{\n",
                "        scale: {:.4}, density: {:.4}, half_len: {:.4}, half_w: {:.5},\n",
                "        color: [{:.4}, {:.4}, {:.4}], brightness: {:.4},\n",
                "    }},\n",
                "    drop: RainDropTuning {{\n",
                "        sway_min: {:.4}, sway_range: {:.4}, fall_spd_base: {:.4}, fall_spd_range: {:.4},\n",
                "        base_y_min: {:.4}, base_y_range: {:.4}, center_x_min: {:.4}, center_x_range: {:.4},\n",
                "        fade_in_end: {:.4}, fade_out_end: {:.4}, shimmer_base: {:.4}, shimmer_amp: {:.4},\n",
                "    }},\n",
                "    mist: RainMistTuning {{\n",
                "        color: [{:.4}, {:.4}, {:.4}],\n",
                "        scale: [{:.4}, {:.4}], scroll: [{:.5}, {:.5}],\n",
                "        soft: [{:.4}, {:.4}], lower: [{:.4}, {:.4}], strength: {:.4},\n",
                "    }},\n",
                "}};\n",
            ),
            self.speed_mul,
            self.scroll_near,
            self.scroll_mid,
            self.scroll_far,
            self.drop_lean,
            self.near.scale,
            self.near.density,
            self.near.half_len,
            self.near.half_w,
            self.near.color[0],
            self.near.color[1],
            self.near.color[2],
            self.near.brightness,
            self.mid.scale,
            self.mid.density,
            self.mid.half_len,
            self.mid.half_w,
            self.mid.color[0],
            self.mid.color[1],
            self.mid.color[2],
            self.mid.brightness,
            self.far.scale,
            self.far.density,
            self.far.half_len,
            self.far.half_w,
            self.far.color[0],
            self.far.color[1],
            self.far.color[2],
            self.far.brightness,
            self.drop.sway_min,
            self.drop.sway_range,
            self.drop.fall_spd_base,
            self.drop.fall_spd_range,
            self.drop.base_y_min,
            self.drop.base_y_range,
            self.drop.center_x_min,
            self.drop.center_x_range,
            self.drop.fade_in_end,
            self.drop.fade_out_end,
            self.drop.shimmer_base,
            self.drop.shimmer_amp,
            self.mist.color[0],
            self.mist.color[1],
            self.mist.color[2],
            self.mist.scale[0],
            self.mist.scale[1],
            self.mist.scroll[0],
            self.mist.scroll[1],
            self.mist.soft[0],
            self.mist.soft[1],
            self.mist.lower[0],
            self.mist.lower[1],
            self.mist.strength,
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
        10..=11 => tuning.near.color,
        17..=18 => tuning.mid.color,
        24..=25 => tuning.far.color,
        38..=39 => tuning.mist.color,
        58..=59 => [
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
    matches!(row, 10 | 17 | 24 | 38 | 58)
}

pub fn rain_row_is_saturation(row: usize) -> bool {
    matches!(row, 11 | 18 | 25 | 39 | 59)
}

/// Full-saturation linear RGB at hue `h` (hue-wheel slider art).
pub fn rain_hue_wheel_preview_linear(hue: f32) -> [f32; 3] {
    hsv_to_rgb_linear(hue.fract(), 1.0, 1.0)
}

pub const RAIN_DEBUG_ROW_META: &[(&str, f32, f32, f32)] = &[
    ("Speed × (all motion)", 0.0, 500.0, 0.1),
    ("Scroll near", 0.0, 50.0, 0.1),
    ("Scroll mid", 0.0, 50.0, 0.1),
    ("Scroll far", 0.0, 50.0, 0.1),
    ("Drop lean", 0.0, 0.25, 0.002),
    ("Near scale", 4.0, 120.0, 1.0),
    ("Near density", 0.0, 1.0, 0.01),
    ("Near half len", 0.02, 0.8, 0.01),
    ("Near half w", 0.001, 0.04, 0.0005),
    ("Near brightness", 0.0, 2.0, 0.02),
    ("Near hue", 0.0, 1.0, 1.0 / 360.0),
    ("Near saturation", 0.0, 1.0, 0.01),
    ("Mid scale", 4.0, 120.0, 1.0),
    ("Mid density", 0.0, 1.0, 0.01),
    ("Mid half len", 0.02, 0.8, 0.01),
    ("Mid half w", 0.001, 0.04, 0.0005),
    ("Mid brightness", 0.0, 2.0, 0.02),
    ("Mid hue", 0.0, 1.0, 1.0 / 360.0),
    ("Mid saturation", 0.0, 1.0, 0.01),
    ("Far scale", 4.0, 120.0, 1.0),
    ("Far density", 0.0, 1.0, 0.01),
    ("Far half len", 0.02, 0.8, 0.01),
    ("Far half w", 0.001, 0.04, 0.0005),
    ("Far brightness", 0.0, 2.0, 0.02),
    ("Far hue", 0.0, 1.0, 1.0 / 360.0),
    ("Far saturation", 0.0, 1.0, 0.01),
    ("Sway min", 0.0, 0.2, 0.002),
    ("Sway range", 0.0, 0.2, 0.002),
    ("Fall spd base", 0.0, 20.0, 0.05),
    ("Fall spd range", 0.0, 20.0, 0.05),
    ("Base Y min", 0.0, 1.0, 0.01),
    ("Base Y range", 0.0, 1.0, 0.01),
    ("Center X min", 0.0, 1.0, 0.01),
    ("Center X range", 0.0, 1.0, 0.01),
    ("Fade in end", 0.01, 0.8, 0.01),
    ("Fade out end", 0.2, 1.0, 0.01),
    ("Shimmer base", 0.0, 1.5, 0.01),
    ("Shimmer amp", 0.0, 1.0, 0.01),
    ("Mist hue", 0.0, 1.0, 1.0 / 360.0),
    ("Mist saturation", 0.0, 1.0, 0.01),
    ("Mist scale X", 0.1, 4.0, 0.02),
    ("Mist scale Y", 0.1, 4.0, 0.02),
    ("Mist scroll X", 0.0, 0.6, 0.005),
    ("Mist scroll Y", 0.0, 0.6, 0.005),
    ("Mist soft lo", 0.0, 1.0, 0.01),
    ("Mist soft hi", 0.0, 1.0, 0.01),
    ("Mist lower start", 0.0, 1.0, 0.01),
    ("Mist lower end", 0.0, 1.0, 0.01),
    ("Mist strength", 0.0, 1.0, 0.01),
    ("Field enabled", 0.0, 1.0, 1.0),
    ("Field spawn rate", 0.0, 800.0, 5.0),
    ("Field pool size", 0.0, 600.0, 10.0),
    ("Field fall (world Z u/s)", 0.0, 3000.0, 20.0),
    ("Field wind X", -800.0, 800.0, 5.0),
    ("Field wind Y", -800.0, 800.0, 5.0),
    ("Field streak px", 2.0, 48.0, 1.0),
    ("Splash count", 1.0, 24.0, 1.0),
    ("Splash lifetime", 0.05, 1.5, 0.02),
    ("Field drop hue", 0.0, 1.0, 1.0 / 360.0),
    ("Field drop saturation", 0.0, 1.0, 0.01),
    ("Field drop alpha", 0.0, 1.0, 0.01),
];

/// Slider rows in the rain debug overlay (Save / Reset / Close follow).
pub const RAIN_DEBUG_SLIDER_COUNT: usize = RAIN_DEBUG_ROW_META.len();
