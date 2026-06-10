//! Volumetric candle flames — digital-garden plume sim, one instance per [`FlameEmitter`].
//!
//! Scenes push emitters (wick anchor, scale, wind, brightness, phase); the renderer
//! calls [`fill_gpu_instances`] and draws via `shaders/flame.wgsl` + [`candle_flame_mesh`].

use glam::{Vec2, Vec3};

// ── Tuning (keep in sync with `shaders/flame.wgsl` / `flame_tuning.rs`) ──

/// Godot base mesh height multiplier — legacy alias; see [`crate::flame_tuning::FlameTuning::flame_height_mul`].
pub const FLAME_MESH_HEIGHT_SCALE: f32 = 4.87;

/// World scale factor applied to candle height for flame emitter size.
pub const FLAME_EMITTER_SCALE_MUL: f32 = 0.22;

/// Typical baked shop candle height in glTF document metres (~52 mm).
pub const SHOP_GLTF_CANDLE_HEIGHT_DOC_M: f32 = 0.052;

/// Nudge flame anchor below `light_candle*` empties (fraction of emitter scale).
pub const SHOP_WICK_BELOW_LIGHT_FRAC: f32 = 0.42;

/// Lightbake height as fraction of shop candle world height.
pub const SHOP_FLAME_LIGHTBAKE_HEIGHT_FRAC: f32 = 0.48;

/// Shop `light_candle*` punctuals + flames — flicker swing at [`FLAME_FLICKER_RATE_HZ`].
pub const SHOP_CANDLE_FLICKER_AMP: f32 = 0.03;

/// Max gust vector length blended into each emitter before upload (shader scales by `wind_strength`).
pub const FLAME_GUST_AMP: f32 = 0.62;
pub const FLAME_GUST_RATE_DEFAULT: f32 = 0.10;
pub const FLAME_GUST_ROOM_MIX_DEFAULT: f32 = 0.35;
const FLAME_GUST_DIR_HZ: f32 = 0.14;
const FLAME_ROOM_GUST_HZ: f32 = 0.07;
const FLAME_GUST_BURST_DECAY: f32 = 1.6;
const FLAME_FLICKER_RATE_HZ: f32 = 24.0;

/// Live gust parameters (from [`crate::flame_tuning::FlameTuning`] / debug menu).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlameGustConfig {
    pub amp: f32,
    pub envelope_hz: f32,
    pub room_mix: f32,
    pub wind_bias: Vec2,
}

impl Default for FlameGustConfig {
    fn default() -> Self {
        Self {
            amp: FLAME_GUST_AMP,
            envelope_hz: FLAME_GUST_RATE_DEFAULT,
            room_mix: FLAME_GUST_ROOM_MIX_DEFAULT,
            wind_bias: Vec2::ZERO,
        }
    }
}

/// Decaying manual gust burst from the debug menu (not persisted).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlameGustRuntime {
    pub per_burst: f32,
    pub room_burst: f32,
    pub burst_dir: Vec2,
    trigger_seq: u32,
}

impl FlameGustRuntime {
    pub fn tick(&mut self, dt_s: f32) {
        let decay = FLAME_GUST_BURST_DECAY * dt_s.max(0.0);
        self.per_burst = (self.per_burst - decay).max(0.0);
        self.room_burst = (self.room_burst - decay).max(0.0);
    }

    /// Fire a one-shot gust. Uses `dir` when non-zero; otherwise picks a pseudo-random heading.
    pub fn trigger(&mut self, dir: Vec2, room: bool, seed: f32) {
        self.trigger_seq = self.trigger_seq.wrapping_add(1);
        let mix_seed = seed + self.trigger_seq as f32 * 0.173 + if room { 17.0 } else { 3.0 };
        self.burst_dir = gust_dir_or_random(dir, mix_seed);
        if room {
            self.room_burst = 1.0;
        } else {
            self.per_burst = 1.0;
        }
    }
}

#[inline]
fn gust_dir_or_random(dir: Vec2, seed: f32) -> Vec2 {
    if dir.length_squared() > 1e-6 {
        return dir.normalize();
    }
    let v = (seed * 12.9898).sin() * 43_758.5453;
    let angle = (v - v.floor()) * std::f32::consts::TAU;
    Vec2::new(angle.cos(), angle.sin())
}

// ── Scene-facing types ───────────────────────────────────────────────────────

/// One candle flame source, supplied by the scene each frame.
#[derive(Clone, Copy, Debug)]
pub struct FlameEmitter {
    /// Wick tip in world space (mesh base at anchor).
    pub wick_world: Vec3,
    /// Uniform world scale for the flame volume.
    pub scale: f32,
    /// Wind bias in world XY; procedural gusts are added at GPU upload (see [`flame_gust_wind`]).
    pub wind: Vec2,
    /// Scene brightness before per-frame flicker (light ramp, lamp flicker, …).
    pub brightness: f32,
    /// Per-candle phase in [0, 1] for desynchronized motion.
    pub phase: f32,
    /// Fast flicker swing (±); rate fixed at [`FLAME_FLICKER_RATE_HZ`].
    pub flicker_amp: f32,
}

/// GPU instance for `flame.wgsl` (matches `@location(3)` / `@location(4)`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuFlameInstance {
    pub anchor: [f32; 3],
    pub wind_x: f32,
    pub scale: f32,
    pub phase: f32,
    pub brightness: f32,
    pub wind_y: f32,
}

impl GpuFlameInstance {
    #[inline]
    pub fn from_emitter(
        e: &FlameEmitter,
        time_s: f32,
        config: &FlameGustConfig,
        manual: &FlameGustRuntime,
    ) -> Self {
        let flick = flame_flicker_multiplier_amp(e.phase, time_s, e.flicker_amp);
        let gust = flame_gust_wind(e.phase, time_s, config, manual);
        let wind = e.wind + gust;
        Self {
            anchor: e.wick_world.to_array(),
            wind_x: wind.x,
            scale: e.scale,
            phase: e.phase,
            brightness: (e.brightness * flick).clamp(0.0, 1.38),
            wind_y: wind.y,
        }
    }
}

/// Fast smoothed 1D noise (random-walk steps). `seed` and `rate_hz` in shader must match.
#[inline]
fn flame_rw_1d(seed: f32, time_s: f32, rate_hz: f32) -> f32 {
    let x = time_s * rate_hz;
    let cell = x.floor();
    let f = x - cell;
    let smooth = f * f * (3.0 - 2.0 * f);
    let h = |cell: f32| {
        let v = (seed * 127.1 + cell * 311.7).sin() * 43_758.547;
        (v - v.floor()) * 2.0 - 1.0
    };
    let a = h(cell);
    let b = h(cell + 1.0);
    a + (b - a) * smooth
}

#[inline]
pub fn shop_candle_flicker_multiplier(phase: f32, time_s: f32) -> f32 {
    flame_flicker_multiplier_amp(phase, time_s, SHOP_CANDLE_FLICKER_AMP)
}

#[inline]
pub fn flame_flicker_multiplier_amp(phase: f32, time_s: f32, amp: f32) -> f32 {
    let seed = phase * 97.13 + 0.17;
    1.0 + flame_rw_1d(seed, time_s, FLAME_FLICKER_RATE_HZ) * amp
}

/// Sparse per-candle gust plus occasional shared room draft — desynced via `phase`.
#[inline]
pub fn flame_gust_wind(
    phase: f32,
    time_s: f32,
    config: &FlameGustConfig,
    manual: &FlameGustRuntime,
) -> Vec2 {
    let amp = config.amp.max(0.0);
    let room_mix = config.room_mix.clamp(0.0, 1.0);
    let per_w = 1.0 - room_mix;
    let room_w = room_mix;

    let auto = if config.envelope_hz > 0.0 {
        let envelope_hz = config.envelope_hz;
        let dir_hz = envelope_hz * (FLAME_GUST_DIR_HZ / FLAME_GUST_RATE_DEFAULT);
        let room_hz = envelope_hz * (FLAME_ROOM_GUST_HZ / FLAME_GUST_RATE_DEFAULT);

        let env_raw = flame_rw_1d(phase * 13.17 + 0.31, time_s, envelope_hz);
        let env = (env_raw * 0.5 + 0.5).clamp(0.0, 1.0).powf(2.6);

        let dir_x = flame_rw_1d(phase * 97.13 + 2.11, time_s, dir_hz);
        let dir_y = flame_rw_1d(phase * 41.07 + 0.17, time_s, dir_hz * 0.91);
        let per = Vec2::new(dir_x, dir_y) * env;

        let room_env_raw = flame_rw_1d(0.731, time_s, room_hz);
        let room_env = (room_env_raw * 0.5 + 0.5).clamp(0.0, 1.0).powf(3.2);
        let room_x = flame_rw_1d(19.71, time_s, room_hz * 0.85);
        let room_y = flame_rw_1d(31.23, time_s, room_hz * 0.77);
        let room = Vec2::new(room_x, room_y) * room_env;

        (per * per_w + room * room_w).clamp_length_max(1.0) * amp
    } else {
        Vec2::ZERO
    };

    let manual_per = manual.burst_dir * manual.per_burst * amp;
    let manual_room = manual.burst_dir * manual.room_burst * amp * 1.1;
    config.wind_bias + auto + manual_per + manual_room * room_w.max(0.35)
}

// ── Scale / placement helpers ────────────────────────────────────────────────

#[inline]
pub fn flame_emitter_scale(candle_world_scale: f32, height_scale: f32) -> f32 {
    candle_world_scale * height_scale * FLAME_EMITTER_SCALE_MUL
}

#[inline]
pub fn shop_gltf_flame_emitter_scale(room_world_scale: f32) -> f32 {
    flame_emitter_scale(
        SHOP_GLTF_CANDLE_HEIGHT_DOC_M * room_world_scale.max(1e-6),
        1.0,
    )
}

#[inline]
pub fn shop_gltf_wick_from_light(light_world: Vec3, emitter_scale: f32) -> Vec3 {
    light_world - Vec3::new(0.0, 0.0, emitter_scale * SHOP_WICK_BELOW_LIGHT_FRAC)
}

#[inline]
pub fn shop_gltf_flame_height_world(room_world_scale: f32) -> f32 {
    SHOP_GLTF_CANDLE_HEIGHT_DOC_M
        * room_world_scale.max(1e-6)
        * SHOP_FLAME_LIGHTBAKE_HEIGHT_FRAC
        * FLAME_MESH_HEIGHT_SCALE
}

/// Upload one [`GpuFlameInstance`] per emitter. Returns instance count.
pub fn fill_gpu_instances(
    emitters: &[FlameEmitter],
    time_s: f32,
    config: &FlameGustConfig,
    manual: &FlameGustRuntime,
    out: &mut Vec<GpuFlameInstance>,
) -> usize {
    out.clear();
    out.extend(emitters.iter().map(|e| {
        GpuFlameInstance::from_emitter(e, time_s, config, manual)
    }));
    out.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_instance_layout_is_32_bytes() {
        assert_eq!(std::mem::size_of::<GpuFlameInstance>(), 32);
    }

    #[test]
    fn flicker_is_bounded() {
        for i in 0..64 {
            let phase = i as f32 / 64.0;
            let f = flame_flicker_multiplier_amp(phase, 1.5, SHOP_CANDLE_FLICKER_AMP);
            assert!(f >= 0.96 && f <= 1.04, "f={f}");
        }
    }

    #[test]
    fn gust_wind_is_bounded() {
        let config = FlameGustConfig::default();
        let manual = FlameGustRuntime::default();
        for i in 0..64 {
            let phase = i as f32 / 64.0;
            for t in [0.0_f32, 1.5, 9.0, 42.0] {
                let w = flame_gust_wind(phase, t, &config, &manual);
                assert!(
                    w.length() <= config.amp + 1e-5,
                    "phase={phase} t={t} len={}",
                    w.length()
                );
            }
        }
    }

    #[test]
    fn gust_wind_varies_over_time() {
        let config = FlameGustConfig::default();
        let manual = FlameGustRuntime::default();
        let w0 = flame_gust_wind(0.31, 0.0, &config, &manual);
        let w1 = flame_gust_wind(0.31, 40.0, &config, &manual);
        assert!(w0.distance(w1) > 0.02, "w0={w0:?} w1={w1:?}");
    }

    #[test]
    fn manual_gust_burst_fades() {
        let config = FlameGustConfig {
            amp: 0.8,
            envelope_hz: 0.0,
            ..Default::default()
        };
        let mut manual = FlameGustRuntime::default();
        manual.trigger(Vec2::new(1.0, 0.0), false, 0.0);
        let strong = flame_gust_wind(0.0, 0.0, &config, &manual);
        assert!(strong.x > 0.5);
        manual.tick(2.0);
        let faded = flame_gust_wind(0.0, 0.0, &config, &manual);
        assert!(faded.length() < strong.length() * 0.2);
    }
}
