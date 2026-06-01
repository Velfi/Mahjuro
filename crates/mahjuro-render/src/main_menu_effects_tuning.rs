//! Persisted tuning for main-menu hub effects (moon, rain, moths).

use crate::main_menu_moon_tuning::MainMenuMoonTuning;
use crate::main_menu_moth_tuning::MainMenuMothTuning;
use crate::rain_tuning::RainTuning;

pub const MAIN_MENU_EFFECTS_SCENE_KEY: &str = crate::scene_keys::MAIN_MENU;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MainMenuEffectsTuning {
    pub rain: RainTuning,
    pub moon: MainMenuMoonTuning,
    pub moths: MainMenuMothTuning,
}

impl Default for MainMenuEffectsTuning {
    fn default() -> Self {
        Self::shipping_default()
    }
}

impl MainMenuEffectsTuning {
    pub fn shipping_default() -> Self {
        Self {
            rain: RainTuning::shipping_default(),
            moon: MainMenuMoonTuning::shipping_default(),
            moths: MainMenuMothTuning::shipping_default(),
        }
    }

    pub fn storage_key() -> String {
        format!("MainMenuEffectsTuning:{MAIN_MENU_EFFECTS_SCENE_KEY}")
    }

    pub fn load() -> Self {
        if mahjuro_gfx_types::has_tuning_override(&Self::storage_key()) {
            return mahjuro_gfx_types::load_tuning_override(&Self::storage_key());
        }
        // Migrate legacy RainTuning-only overrides (included moon emission color).
        let legacy_key = RainTuning::storage_key();
        if mahjuro_gfx_types::has_tuning_override(&legacy_key) {
            let legacy: LegacyRainTuningFile = mahjuro_gfx_types::load_tuning_override(&legacy_key);
            return Self {
                rain: RainTuning {
                    speed_mul: legacy.speed_mul,
                    field: legacy.field,
                },
                moon: MainMenuMoonTuning {
                    emission_color: legacy.moon_emission_color,
                },
                moths: MainMenuMothTuning::shipping_default(),
            };
        }
        Self::shipping_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        mahjuro_gfx_types::save_tuning_override(&Self::storage_key(), self)
    }

    pub fn clear_saved() -> anyhow::Result<()> {
        mahjuro_gfx_types::clear_tuning_override(&Self::storage_key())
    }

    pub fn to_rust_literal(self) -> String {
        format!(
            concat!(
                "// MainMenuEffectsTuning snapshot\n",
                "use crate::main_menu_effects_tuning::MainMenuEffectsTuning;\n",
                "use crate::main_menu_moon_tuning::MainMenuMoonTuning;\n",
                "use crate::main_menu_moth_tuning::MainMenuMothTuning;\n",
                "use crate::rain_tuning::{{RainFieldTuning, RainTuning}};\n",
                "const MAIN_MENU_EFFECTS: MainMenuEffectsTuning = MainMenuEffectsTuning {{\n",
                "    rain: RainTuning {{\n",
                "        speed_mul: {:.4},\n",
                "        field: RainFieldTuning {{\n",
                "            density: {:.4}, spawn_rate: {:.4}, pool_size: {:.4}, fall_speed: {:.4},\n",
                "            wind_x: {:.4}, wind_y: {:.4}, streak_len_px: {:.4}, splash_count: {:.4},\n",
                "            splash_lifetime: {:.4},\n",
                "            drop_color: [{:.4}, {:.4}, {:.4}, {:.4}],\n",
                "            volume_pad_xy: {:.4}, volume_top_mul: {:.4},\n",
                "            spawn_near_bias: {:.4},\n",
                "        }},\n",
                "    }},\n",
                "    moon: MainMenuMoonTuning {{\n",
                "        emission_color: [{:.4}, {:.4}, {:.4}],\n",
                "    }},\n",
                "    moths: MainMenuMothTuning {{\n",
                "        flap_hz: {:.4}, flap_amp: {:.4}, body_size_mul: {:.4},\n",
                "        orbit_radius_mul: {:.4}, orbit_speed_mul: {:.4}, bob_amp_mul: {:.4},\n",
                "    }},\n",
                "}};\n",
            ),
            self.rain.speed_mul,
            self.rain.field.density,
            self.rain.field.spawn_rate,
            self.rain.field.pool_size,
            self.rain.field.fall_speed,
            self.rain.field.wind_x,
            self.rain.field.wind_y,
            self.rain.field.streak_len_px,
            self.rain.field.splash_count,
            self.rain.field.splash_lifetime,
            self.rain.field.drop_color[0],
            self.rain.field.drop_color[1],
            self.rain.field.drop_color[2],
            self.rain.field.drop_color[3],
            self.rain.field.volume_pad_xy,
            self.rain.field.volume_top_mul,
            self.rain.field.spawn_near_bias,
            self.moon.emission_color[0],
            self.moon.emission_color[1],
            self.moon.emission_color[2],
            self.moths.flap_hz,
            self.moths.flap_amp,
            self.moths.body_size_mul,
            self.moths.orbit_radius_mul,
            self.moths.orbit_speed_mul,
            self.moths.bob_amp_mul,
        )
    }
}

#[derive(serde::Deserialize)]
struct LegacyRainTuningFile {
    speed_mul: f32,
    field: crate::rain_tuning::RainFieldTuning,
    #[serde(default = "default_moon_emission_color")]
    moon_emission_color: [f32; 3],
}

impl Default for LegacyRainTuningFile {
    fn default() -> Self {
        let rain = RainTuning::shipping_default();
        Self {
            speed_mul: rain.speed_mul,
            field: rain.field,
            moon_emission_color: default_moon_emission_color(),
        }
    }
}

fn default_moon_emission_color() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipping_default_round_trips_fields() {
        let t = MainMenuEffectsTuning::shipping_default();
        assert!(t.rain.speed_mul > 0.0);
        assert_eq!(t.moon.emission_color, [1.0, 1.0, 1.0]);
        assert_eq!(t.moths.flap_hz, 25.0);
    }
}
