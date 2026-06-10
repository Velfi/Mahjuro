//! Main-menu exponential height fog tuning.

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MainMenuFogTuning {
    /// Optical depth over one viewport-height ray at floor level.
    #[serde(default = "default_density")]
    pub density: f32,
    /// Exponential falloff height as a fraction of viewport height.
    #[serde(default = "default_height")]
    pub height: f32,
    /// World-space fog floor lift in multiples of [`Self::height_world`].
    #[serde(default = "default_floor_lift_heights")]
    pub floor_lift_heights: f32,
    /// Linear HDR fog target color before [`Self::brightness`].
    #[serde(default = "default_color")]
    pub color: [f32; 3],
    /// Multiplier on [`Self::color`] in shader linear HDR space.
    #[serde(default = "default_brightness")]
    pub brightness: f32,
}

fn default_density() -> f32 {
    7.0
}

fn default_height() -> f32 {
    0.055
}

fn default_floor_lift_heights() -> f32 {
    2.0
}

fn default_color() -> [f32; 3] {
    [0.46, 0.52, 0.58]
}

fn default_brightness() -> f32 {
    1.0
}

impl Default for MainMenuFogTuning {
    fn default() -> Self {
        Self::shipping_default()
    }
}

impl MainMenuFogTuning {
    pub fn shipping_default() -> Self {
        Self {
            density: default_density(),
            height: default_height(),
            floor_lift_heights: default_floor_lift_heights(),
            color: default_color(),
            brightness: default_brightness(),
        }
    }

    pub fn density_per_world_unit(self, window_h: f32) -> f32 {
        self.density.max(0.0) / window_h.max(1.0)
    }

    pub fn height_world(self, window_h: f32) -> f32 {
        self.height.max(0.001) * window_h.max(1.0)
    }

    pub fn floor_lift_world(self, window_h: f32) -> f32 {
        self.height_world(window_h) * self.floor_lift_heights
    }

    pub fn color_hdr(self) -> [f32; 3] {
        let b = self.brightness.max(0.0);
        [self.color[0] * b, self.color[1] * b, self.color[2] * b]
    }

    pub fn debug_row_value(self, row: usize) -> f32 {
        match row {
            0 => self.density,
            1 => self.height,
            2 => self.floor_lift_heights,
            3 => crate::rain_tuning::rgb_linear_hue(self.color),
            4 => crate::rain_tuning::rgb_linear_sat(self.color),
            5 => self.brightness,
            _ => 0.0,
        }
    }

    pub fn set_debug_row_value(&mut self, row: usize, v: f32) {
        let (_, lo, hi, _) = FOG_DEBUG_ROW_META[row];
        let v = v.clamp(lo, hi);
        match row {
            0 => self.density = v,
            1 => self.height = v,
            2 => self.floor_lift_heights = v,
            3 => crate::rain_tuning::set_linear_rgb_hue(&mut self.color, v),
            4 => crate::rain_tuning::set_linear_rgb_sat(&mut self.color, v),
            5 => self.brightness = v,
            _ => {}
        }
    }

    pub fn color_swatch_rgb(self, row: usize) -> Option<[f32; 3]> {
        match row {
            3..=5 => {
                let rgb = self.color;
                let peak = rgb[0].max(rgb[1]).max(rgb[2]);
                if peak <= 1e-8 {
                    Some([0.0; 3])
                } else {
                    Some([rgb[0] / peak, rgb[1] / peak, rgb[2] / peak])
                }
            }
            _ => None,
        }
    }
}

pub const FOG_DEBUG_ROW_META: &[(&str, f32, f32, f32)] = &[
    ("Density", 0.0, 12.0, 0.1),
    ("Height", 0.02, 0.50, 0.01),
    ("Floor lift (x height)", -2.0, 6.0, 0.05),
    ("Fog hue", 0.0, 1.0, 1.0 / 360.0),
    ("Fog saturation", 0.0, 1.0, 0.01),
    ("Fog brightness", 0.0, 3.0, 0.02),
];

pub const FOG_DEBUG_SLIDER_COUNT: usize = FOG_DEBUG_ROW_META.len();

pub fn fog_row_is_hue(row: usize) -> bool {
    row == 3
}

pub fn fog_row_is_saturation(row: usize) -> bool {
    row == 4
}
