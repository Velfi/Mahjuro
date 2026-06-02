//! Main-menu hub moon / star emissive tint (debug overlay Moon tab).

use crate::wgpu_renderer::{current_moon_phase, main_menu_moon_phase_for_render};

/// Live calendar vs forced synodic phase (not saved with emission tuning).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MainMenuMoonPhaseDebug {
    pub use_live_calendar: bool,
    pub forced_phase: f32,
}

impl Default for MainMenuMoonPhaseDebug {
    fn default() -> Self {
        Self {
            use_live_calendar: true,
            forced_phase: 0.0,
        }
    }
}

impl MainMenuMoonPhaseDebug {
    pub fn resolved_phase(self) -> f32 {
        main_menu_moon_phase_for_render(self.use_live_calendar, self.forced_phase)
    }

    pub fn sync_forced_from_calendar(&mut self) {
        self.forced_phase = current_moon_phase();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MainMenuMoonTuning {
    /// Linear RGB multiplier on `MoonObject` / `star*` glTF emissive (after authored strength).
    #[serde(default = "default_moon_emission_color")]
    pub emission_color: [f32; 3],
}

fn default_moon_emission_color() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

impl Default for MainMenuMoonTuning {
    fn default() -> Self {
        Self::shipping_default()
    }
}

impl MainMenuMoonTuning {
    pub fn shipping_default() -> Self {
        Self {
            emission_color: default_moon_emission_color(),
        }
    }

    pub fn debug_row_value(self, row: usize) -> f32 {
        match row {
            0 => crate::rain_tuning::rgb_linear_hue(self.emission_color),
            1 => crate::rain_tuning::rgb_linear_sat(self.emission_color),
            _ => 0.0,
        }
    }

    pub fn set_debug_row_value(&mut self, row: usize, v: f32) {
        let (_, lo, hi, _) = MOON_DEBUG_ROW_META[row];
        let v = v.clamp(lo, hi);
        match row {
            0 => {
                let mut rgb = self.emission_color;
                crate::rain_tuning::set_linear_rgb_hue(&mut rgb, v);
                self.emission_color = rgb;
            }
            1 => {
                let mut rgb = self.emission_color;
                crate::rain_tuning::set_linear_rgb_sat(&mut rgb, v);
                self.emission_color = rgb;
            }
            _ => {}
        }
    }

    pub fn color_swatch_rgb(self, row: usize) -> Option<[f32; 3]> {
        match row {
            0..=1 => {
                let rgb = self.emission_color;
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

pub const MOON_DEBUG_ROW_META: &[(&str, f32, f32, f32)] = &[
    ("Moon / star emission hue", 0.0, 1.0, 1.0 / 360.0),
    ("Moon / star emission saturation", 0.0, 1.0, 0.01),
    ("Moon phase (synodic)", 0.0, 1.0, 0.01),
];

pub const MOON_DEBUG_SLIDER_COUNT: usize = MOON_DEBUG_ROW_META.len();

pub fn moon_row_is_hue(row: usize) -> bool {
    row == 0
}

pub fn moon_row_is_saturation(row: usize) -> bool {
    row == 1
}

pub fn moon_row_is_phase(row: usize) -> bool {
    row == 2
}
