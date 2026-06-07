//! Victory run-summary 3D moon rotation + synodic phase (debug overlay).

use crate::main_menu_glb::VICTORY_MOON_ROTATION_XYZ;
use crate::main_menu_moon_tuning::MainMenuMoonPhaseDebug;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VictoryMoonDebug {
    /// Euler XYZ radians applied after recentering / scale on the victory moon mesh.
    pub rotation_xyz: [f32; 3],
    pub moon_phase: MainMenuMoonPhaseDebug,
}

impl Default for VictoryMoonDebug {
    fn default() -> Self {
        Self::shipping_default()
    }
}

impl VictoryMoonDebug {
    pub fn shipping_default() -> Self {
        Self {
            rotation_xyz: VICTORY_MOON_ROTATION_XYZ,
            moon_phase: MainMenuMoonPhaseDebug::default(),
        }
    }

    pub fn debug_row_value(self, row: usize) -> f32 {
        match row {
            0..=2 => self.rotation_xyz[row].to_degrees(),
            3 => {
                if self.moon_phase.use_live_calendar {
                    self.moon_phase.resolved_phase()
                } else {
                    self.moon_phase.forced_phase
                }
            }
            _ => 0.0,
        }
    }

    pub fn set_debug_row_value(&mut self, row: usize, v: f32) {
        match row {
            0..=2 => {
                let (_, lo, hi, _) = VICTORY_MOON_DEBUG_ROW_META[row];
                self.rotation_xyz[row] = v.clamp(lo, hi).to_radians();
            }
            3 => {
                self.moon_phase.use_live_calendar = false;
                self.moon_phase.forced_phase = v.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    pub fn set_moon_phase_preset(&mut self, phase: f32) {
        self.moon_phase.use_live_calendar = false;
        self.moon_phase.forced_phase = phase.clamp(0.0, 1.0);
    }

    pub fn toggle_live_calendar(&mut self) {
        self.moon_phase.use_live_calendar = !self.moon_phase.use_live_calendar;
        if !self.moon_phase.use_live_calendar {
            self.moon_phase.sync_forced_from_calendar();
        }
    }

    pub fn to_rust_literal(self) -> String {
        format!(
            "/// Euler XYZ radians for the victory run-summary 3D moon.\n\
             pub const VICTORY_MOON_ROTATION_XYZ: [f32; 3] = [{:.6}, {:.6}, {:.6}];",
            self.rotation_xyz[0], self.rotation_xyz[1], self.rotation_xyz[2],
        )
    }
}

pub const VICTORY_MOON_DEBUG_ROW_META: &[(&str, f32, f32, f32)] = &[
    ("Rotation X (°)", -180.0, 180.0, 1.0),
    ("Rotation Y (°)", -180.0, 180.0, 1.0),
    ("Rotation Z (°)", -180.0, 180.0, 1.0),
    ("Moon phase (synodic)", 0.0, 1.0, 0.01),
];

pub const VICTORY_MOON_DEBUG_SLIDER_COUNT: usize = VICTORY_MOON_DEBUG_ROW_META.len();

/// Action rows after sliders: live toggle, four presets, copy, reset, close.
pub const VICTORY_MOON_DEBUG_ACTION_COUNT: usize = 8;

pub const VICTORY_MOON_DEBUG_ROW_COUNT: usize =
    VICTORY_MOON_DEBUG_SLIDER_COUNT + VICTORY_MOON_DEBUG_ACTION_COUNT;

pub const VICTORY_MOON_ACTION_LIVE: usize = VICTORY_MOON_DEBUG_SLIDER_COUNT;
pub const VICTORY_MOON_ACTION_NEW: usize = VICTORY_MOON_DEBUG_SLIDER_COUNT + 1;
pub const VICTORY_MOON_ACTION_FIRST_QUARTER: usize = VICTORY_MOON_DEBUG_SLIDER_COUNT + 2;
pub const VICTORY_MOON_ACTION_FULL: usize = VICTORY_MOON_DEBUG_SLIDER_COUNT + 3;
pub const VICTORY_MOON_ACTION_LAST_QUARTER: usize = VICTORY_MOON_DEBUG_SLIDER_COUNT + 4;
pub const VICTORY_MOON_ACTION_COPY: usize = VICTORY_MOON_DEBUG_SLIDER_COUNT + 5;
pub const VICTORY_MOON_ACTION_RESET: usize = VICTORY_MOON_DEBUG_SLIDER_COUNT + 6;
pub const VICTORY_MOON_ACTION_CLOSE: usize = VICTORY_MOON_DEBUG_SLIDER_COUNT + 7;
