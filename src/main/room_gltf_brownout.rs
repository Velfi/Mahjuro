//! Rare random bulb flicker + brief brownout for room-glTF scenes (shop,
//! pick_blind, collection). Also fires when the player skips a blind.
//! Ticks only while that scene is on-screen and nothing is blocking
//! gameplay-style overlays (pause, shop env debug, …).

use crate::render::room_glb::RoomEnvLightingTune;
use crate::scenes::Scene;
use rand::RngExt;

/// Between events while in an eligible scene (roughly one event per ~5 min).
const IDLE_MIN_SECS: f32 = 240.0;
const IDLE_MAX_SECS: f32 = 420.0;

#[derive(Clone, Debug)]
pub struct RoomGltfBrownout {
    secs_to_next: f32,
    active: Option<ActiveBrownout>,
}

#[derive(Clone, Debug)]
struct ActiveBrownout {
    t: f32,
    flicker_secs: f32,
    dip_secs: f32,
    recover_secs: f32,
    dip_severity: f32,
}

impl RoomGltfBrownout {
    pub fn new() -> Self {
        Self {
            secs_to_next: Self::roll_idle_delay(),
            active: None,
        }
    }

    fn roll_idle_delay() -> f32 {
        let mut rng = rand::rng();
        rng.random_range(IDLE_MIN_SECS..IDLE_MAX_SECS)
    }

    fn roll_active() -> ActiveBrownout {
        let mut rng = rand::rng();
        ActiveBrownout {
            t: 0.0,
            flicker_secs: rng.random_range(1.05_f32..1.85_f32),
            dip_secs: rng.random_range(0.38_f32..0.72_f32),
            recover_secs: rng.random_range(0.42_f32..0.88_f32),
            dip_severity: rng.random_range(0.58_f32..1.0_f32),
        }
    }

    #[inline]
    fn total_event_secs(a: &ActiveBrownout) -> f32 {
        a.flicker_secs + a.dip_secs + a.recover_secs
    }

    /// Start flicker + dip immediately (e.g. player skipped a blind on pick_blind).
    pub fn trigger(&mut self) {
        self.active = Some(Self::roll_active());
        self.secs_to_next = 0.0;
    }

    /// `freeze`: pause menu, shop lighting debug overlay, or scene blocking overlay.
    pub fn tick(&mut self, dt: f32, room_scene_eligible: bool, freeze: bool) {
        if !room_scene_eligible {
            self.active = None;
            return;
        }
        if freeze {
            return;
        }

        if let Some(ref mut a) = self.active {
            a.t += dt;
            if a.t >= Self::total_event_secs(a) {
                self.active = None;
                self.secs_to_next = Self::roll_idle_delay();
            }
        } else {
            self.secs_to_next -= dt;
            if self.secs_to_next <= 0.0 {
                self.active = Some(Self::roll_active());
                self.secs_to_next = 0.0;
            }
        }
    }

    #[inline]
    pub fn scene_eligible(scene: &Scene) -> bool {
        matches!(
            scene,
            Scene::Shop(_) | Scene::PickBlind(_) | Scene::Collection(_)
        )
    }

    #[inline]
    fn smoothstep01(x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    }

    fn dip_full(sev: f32) -> (f32, f32, f32, f32, f32, f32) {
        let k = 1.0_f32;
        let bulb = (1.0 - 0.82 * sev * k).max(0.08);
        let exp = (1.0 - 0.62 * sev * k).max(0.26);
        let amb = (1.0 - 0.55 * sev * k).max(0.24);
        let em = (1.0 - 0.42 * sev * k).max(0.38);
        let punct = (1.0 - 0.48 * sev * k).max(0.28);
        let candle = (1.0 - 0.32 * sev * k).max(0.68);
        (bulb, exp, amb, em, punct, candle)
    }

    #[inline]
    fn lerp_f(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }

    /// Multiply punctual / exposure / emissive tuning. Inactive → `base` unchanged.
    pub fn apply(&self, base: RoomEnvLightingTune) -> RoomEnvLightingTune {
        let Some(ref a) = self.active else {
            return base;
        };

        let tf = a.flicker_secs;
        let td = a.dip_secs;
        let tr = a.recover_secs;
        let t = a.t;
        let sev = a.dip_severity;

        let (gltf_mul, exp_mul, amb_mul, emissive_mul, punct_mul, candle_dim) = if t < tf {
            let ft = t;
            let fast1 = (ft * 49.0).sin();
            let fast2 = (ft * 21.0 + 0.7).sin();
            let slow = (ft * 2.6).sin() * 0.07;
            let chaos = 0.56 + 0.36 * (0.5 + 0.5 * fast1) * (0.5 + 0.5 * fast2) + slow;
            let spike = if (ft * 13.1).fract() < 0.035 {
                0.16
            } else {
                1.0
            };
            let gltf = (chaos * spike).clamp(0.09, 1.22);
            let exp = (0.94 + 0.06 * fast2.abs()).clamp(0.88, 1.0);
            (gltf, exp, 1.0, 1.0, 1.0, 1.0)
        } else if t < tf + td {
            let u = ((t - tf) / td).clamp(0.0, 1.0);
            let dip_k = Self::smoothstep01(u);
            let (b0, e0, a0, em0, p0, c0) = Self::dip_full(sev);
            let gltf = Self::lerp_f(1.0, b0, dip_k);
            let exp = Self::lerp_f(1.0, e0, dip_k);
            let amb = Self::lerp_f(1.0, a0, dip_k);
            let em = Self::lerp_f(1.0, em0, dip_k);
            let punct = Self::lerp_f(1.0, p0, dip_k);
            let candle = Self::lerp_f(1.0, c0, dip_k);
            (gltf, exp, amb, em, punct, candle)
        } else {
            let u = ((t - tf - td) / tr).clamp(0.0, 1.0);
            let rec = Self::smoothstep01(u);
            let (b0, e0, a0, em0, p0, c0) = Self::dip_full(sev);
            let gltf = Self::lerp_f(b0, 1.0, rec);
            let exp = Self::lerp_f(e0, 1.0, rec);
            let amb = Self::lerp_f(a0, 1.0, rec);
            let em = Self::lerp_f(em0, 1.0, rec);
            let punct = Self::lerp_f(p0, 1.0, rec);
            let candle = Self::lerp_f(c0, 1.0, rec);
            (gltf, exp, amb, em, punct, candle)
        };

        RoomEnvLightingTune {
            gltf_light_intensity_scale: base.gltf_light_intensity_scale * gltf_mul,
            linear_exposure: base.linear_exposure * exp_mul,
            ambient_scale: base.ambient_scale * amb_mul,
            lit_mesh_gltf_punctual_scale: base.lit_mesh_gltf_punctual_scale * punct_mul,
            gltf_emissive_scale: base.gltf_emissive_scale * emissive_mul,
            candle_light_color_mul: [
                base.candle_light_color_mul[0] * candle_dim,
                base.candle_light_color_mul[1] * candle_dim,
                base.candle_light_color_mul[2] * candle_dim,
            ],
            lantern_light_color_mul: [
                base.lantern_light_color_mul[0] * candle_dim,
                base.lantern_light_color_mul[1] * candle_dim,
                base.lantern_light_color_mul[2] * candle_dim,
            ],
        }
    }
}
