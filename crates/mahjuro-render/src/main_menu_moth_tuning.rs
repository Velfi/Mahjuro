//! Main-menu door-light moth motion tuning (debug overlay Moths tab).

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MainMenuMothTuning {
    /// Wingbeat frequency (Hz).
    pub flap_hz: f32,
    /// Peak wing flap angle multiplier (rad scale on sine).
    pub flap_amp: f32,
    /// Scales body length (`window_h * 0.003`).
    pub body_size_mul: f32,
    /// Scales orbit radius from lamp width fractions.
    pub orbit_radius_mul: f32,
    /// Scales per-bug orbit speed from authored [`BUG_PARAMS`] fractions.
    pub orbit_speed_mul: f32,
    /// Scales vertical bob amplitude around the lamp anchor.
    pub bob_amp_mul: f32,
}

impl Default for MainMenuMothTuning {
    fn default() -> Self {
        Self::shipping_default()
    }
}

impl MainMenuMothTuning {
    pub fn shipping_default() -> Self {
        Self {
            flap_hz: 25.0,
            flap_amp: 0.82,
            body_size_mul: 1.0,
            orbit_radius_mul: 1.0,
            orbit_speed_mul: 1.0,
            bob_amp_mul: 1.0,
        }
    }

    pub fn debug_row_value(self, row: usize) -> f32 {
        match row {
            0 => self.flap_hz,
            1 => self.flap_amp,
            2 => self.body_size_mul,
            3 => self.orbit_radius_mul,
            4 => self.orbit_speed_mul,
            5 => self.bob_amp_mul,
            _ => 0.0,
        }
    }

    pub fn set_debug_row_value(&mut self, row: usize, v: f32) {
        let (_, lo, hi, _) = MOTH_DEBUG_ROW_META[row];
        let v = v.clamp(lo, hi);
        match row {
            0 => self.flap_hz = v,
            1 => self.flap_amp = v,
            2 => self.body_size_mul = v,
            3 => self.orbit_radius_mul = v,
            4 => self.orbit_speed_mul = v,
            5 => self.bob_amp_mul = v,
            _ => {}
        }
    }
}

pub const MOTH_DEBUG_ROW_META: &[(&str, f32, f32, f32)] = &[
    ("Flap rate (Hz)", 4.0, 60.0, 0.5),
    ("Flap amplitude", 0.05, 1.5, 0.02),
    ("Body size ×", 0.25, 3.0, 0.05),
    ("Orbit radius ×", 0.25, 2.5, 0.02),
    ("Orbit speed ×", 0.1, 4.0, 0.05),
    ("Bob amplitude ×", 0.0, 3.0, 0.05),
];

pub const MOTH_DEBUG_SLIDER_COUNT: usize = MOTH_DEBUG_ROW_META.len();
