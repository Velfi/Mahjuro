//! Volumetric candle flames — one revolved mesh instance per [`FlameEmitter`].
//!
//! Scenes push emitters (wick anchor, scale, wind, brightness, phase); the renderer
//! calls [`fill_gpu_instances`] and draws via `shaders/flame.wgsl` + [`candle_flame_mesh`].

use glam::{Vec2, Vec3};

// ── Tuning (keep in sync with `shaders/flame.wgsl` / `shaders/blackbody.wgsl`) ──

/// Godot base mesh height multiplier in `flame.wgsl` (`FLAME_HEIGHT = BASE * SCALE`).
#[allow(dead_code)]
pub const FLAME_MESH_HEIGHT_BASE: f32 = 1.72;
pub const FLAME_MESH_HEIGHT_SCALE: f32 = 4.0;
#[allow(dead_code)]
pub const FLAME_MESH_WIDTH_MUL: f32 = 1.64;

/// World scale factor applied to candle height for flame emitter size.
pub const FLAME_EMITTER_SCALE_MUL: f32 = 0.22;

/// Typical baked shop candle height in glTF document metres (~52 mm).
pub const SHOP_GLTF_CANDLE_HEIGHT_DOC_M: f32 = 0.052;

/// Nudge flame anchor below `light_candle*` empties (fraction of emitter scale).
pub const SHOP_WICK_BELOW_LIGHT_FRAC: f32 = 0.42;

/// Lightbake height as fraction of shop candle world height.
pub const SHOP_FLAME_LIGHTBAKE_HEIGHT_FRAC: f32 = 0.48;

/// Shop `light_candle*` punctuals + flames — flicker swing at [`FLAME_FLICKER_RATE_HZ`].
pub const SHOP_CANDLE_FLICKER_AMP: f32 = 0.019;
const FLAME_FLICKER_RATE_HZ: f32 = 24.0;

// ── Scene-facing types ───────────────────────────────────────────────────────

/// One candle flame source, supplied by the scene each frame.
#[derive(Clone, Copy, Debug)]
pub struct FlameEmitter {
    /// Wick tip in world space (mesh base at anchor).
    pub wick_world: Vec3,
    /// Uniform world scale for the flame volume.
    pub scale: f32,
    /// Wind bias in world XY (gameplay gusts); shader clamps contribution.
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
    pub fn from_emitter(e: &FlameEmitter, time_s: f32) -> Self {
        let flick = flame_flicker_multiplier_amp(e.phase, time_s, e.flicker_amp);
        Self {
            anchor: e.wick_world.to_array(),
            wind_x: e.wind.x,
            scale: e.scale,
            phase: e.phase,
            brightness: (e.brightness * flick).clamp(0.0, 1.38),
            wind_y: e.wind.y,
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
    out: &mut Vec<GpuFlameInstance>,
) -> usize {
    out.clear();
    out.extend(
        emitters
            .iter()
            .map(|e| GpuFlameInstance::from_emitter(e, time_s)),
    );
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
}
