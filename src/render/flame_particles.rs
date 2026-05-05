//! CPU-side flame particle pool for the 3D flame renderer.
//!
//! Replaces the single 2D additive quad per candle with a small cloud of
//! billboarded 3D particles that rise from the wick, are carved by noise,
//! and dissolve by age. The look is a port of the Godot "stylized flame"
//! particle shader: each particle samples a radial shape, a distortion
//! field, and a dissolve noise, then burns out along an age curve.
//!
//! ### Architecture
//!
//! The renderer owns one [`FlameParticleSystem`]. Each frame:
//!
//! 1. Scenes push one `DrawCmd::Flame` per candle (as before); the
//!    gameplay draw loop projects each candle's wick tip into world space
//!    and supplies that as the emitter anchor.
//! 2. The renderer calls [`FlameParticleSystem::step`] with emitters,
//!    frame delta, and a monotonic time (seconds) for shared flicker/curl.
//!    The pool ages/spawns/despawns particles.
//! 3. The renderer uploads the live particle array into a per-frame
//!    instance buffer and dispatches the 3D flame pipeline once.
//!
//! Particle state is intentionally tiny (≈ 64 bytes) and the pool is
//! capped so we never churn GPU memory.

use glam::{Vec2, Vec3};

/// One candle's emission source, supplied by the scene each frame.
#[derive(Clone, Copy, Debug)]
pub struct FlameEmitter {
    /// World-space position of the wick tip — where particles spawn.
    pub wick_world: Vec3,
    /// Uniform scale the candle was rendered at. Particles inherit it so a
    /// jittered-larger candle gets a proportionally larger flame.
    pub scale: f32,
    /// Flame-relative wind vector (≈ [-1.5, 1.5]) from the renderer's
    /// gust sampler. x pushes the particle horizontally in world X; y is
    /// a small downward squash when negative.
    pub wind: Vec2,
    /// Brightness multiplier in [0, 1]. Gameplay ramps this for "light
    /// ramp" + candle-flare beats.
    pub brightness: f32,
    /// Per-candle phase offset in [0, 1] so neighbouring candles don't
    /// beat in sync.
    pub phase: f32,
}

/// Per-particle live state.
#[derive(Clone, Copy, Debug)]
struct Particle {
    /// World-space position.
    pos: Vec3,
    /// World-space velocity (world units / sec). Mostly +Z (up).
    vel: Vec3,
    /// Seconds since spawn.
    age: f32,
    /// Total lifetime (seconds).
    lifetime: f32,
    /// Spawn scale (world units). Particles keep this — the dissolve eats
    /// them, not a scale ramp.
    scale: f32,
    /// Per-particle random phase in [0, 2π] — drives noise phase offsets
    /// in the fragment shader so particles don't alias.
    phase: f32,
    /// Index of the emitter that spawned this particle. Used to inherit
    /// per-frame wind and brightness updates.
    emitter: u16,
    /// True once the particle has passed its lifetime; the pool overwrites
    /// it on the next spawn.
    dead: bool,
}

impl Particle {
    fn dead_slot() -> Self {
        Self {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            age: 0.0,
            lifetime: 0.0,
            scale: 0.0,
            phase: 0.0,
            emitter: 0,
            dead: true,
        }
    }
}

/// GPU-facing per-particle instance. The vertex shader reads this for each
/// billboarded quad (4 verts). Keep tightly packed and `bytemuck`-safe.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuFlameParticle {
    /// World-space center of the billboard.
    pub pos: [f32; 3],
    /// Normalized age (0 = just spawned, 1 = dying). Drives the dissolve
    /// threshold in the fragment shader.
    pub age: f32,
    /// Billboard half-extent in world units (width = height).
    pub scale: f32,
    /// Per-particle random phase in [0, 2π].
    pub phase: f32,
    /// Brightness multiplier inherited from the emitter at spawn time +
    /// live-updated each frame; drives an emission scalar in the frag.
    pub brightness: f32,
    /// 0 = camera-facing billboard plane; 1 = second plane rotated 90°
    /// around world +Z (cross billboards) so the plume reads volumetric.
    pub cross_slice: f32,
}

/// Maximum live particles across *all* candles. 7 candles × ~24 particles
/// = 168 typical; cap at 256 so an over-eager spawn rate can't OOM.
const MAX_PARTICLES: usize = 256;

/// Particles a single emitter is allowed to have live at once. The
/// billboards are narrow (0.75× width, 2.4× height of the particle
/// scale) so a candle needs ~30–36 overlapping particles to read as a
/// continuous plume rather than a chain of puffs.
const PER_EMITTER_CAP: usize = 36;

/// Particles spawned per second per emitter. With a lifetime ~0.95s and
/// this rate, the steady-state count lands near `PER_EMITTER_CAP`.
const SPAWN_RATE_HZ: f32 = 36.0;

/// Mean particle lifetime. The flame's vertical reach scales with
/// lifetime × vertical velocity, so don't push it without also adjusting
/// the wick-tip-to-flame-tip ratio in [`crate::render::candle_mesh`].
const LIFETIME_MEAN: f32 = 0.95;
const LIFETIME_JITTER: f32 = 0.25;

/// Upward velocity at spawn (world units / sec). This sets the flame's
/// nominal height: `LIFETIME_MEAN * RISE_SPEED` is roughly the distance
/// a surviving particle travels before dissolving.
const RISE_SPEED: f32 = 34.0;

/// How aggressively wind tips the particle trajectory. The flame shader
/// also bends per-pixel (for silhouette deformation); this is the
/// bulk-motion component that makes the whole plume lean.
const WIND_ACCEL: f32 = 22.0;

/// Lateral curl / flicker in world XY — uses wall-clock time + emitter
/// phase so neighbouring candles stay decorrelated but each plume wobbles
/// coherently.
const CURL_ACCEL: f32 = 26.0;

pub struct FlameParticleSystem {
    particles: Vec<Particle>,
    /// Per-emitter spawn-timer residual so emission is smooth across
    /// variable frame deltas (no bursty spawns at low frame rate).
    spawn_accum: Vec<f32>,
    /// Per-emitter live count so we enforce [`PER_EMITTER_CAP`] without
    /// scanning the pool each spawn.
    per_emitter_count: Vec<u32>,
    /// Tiny PRNG state — a single LCG is more than enough for ~300
    /// particles/sec of jitter and keeps spawn determinism easy to
    /// reason about across frames.
    rng_state: u32,
}

impl Default for FlameParticleSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FlameParticleSystem {
    pub fn new() -> Self {
        Self {
            particles: vec![Particle::dead_slot(); MAX_PARTICLES],
            spawn_accum: Vec::new(),
            per_emitter_count: Vec::new(),
            rng_state: 0x1337BEEF,
        }
    }

    fn rand01(&mut self) -> f32 {
        // Standard 32-bit LCG (Numerical Recipes). Bit-twiddle the top bits
        // to pull uniform f32 in [0,1).
        self.rng_state = self
            .rng_state
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
        (self.rng_state >> 8) as f32 / ((1u32 << 24) as f32)
    }

    fn rand_range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rand01()
    }

    /// Step the particle system by `dt` seconds, spawning new particles
    /// from each emitter and aging/despawning existing ones.
    ///
    /// Returns the number of live particles after the step (bounded by
    /// [`MAX_PARTICLES`]).
    pub fn step(&mut self, emitters: &[FlameEmitter], dt: f32, time_s: f32) -> usize {
        if dt <= 0.0 {
            return self.live_count();
        }
        let dt = dt.min(0.05); // clamp: long hitches shouldn't dump a burst.

        // Resize per-emitter bookkeeping. A shrinking emitter list (e.g.
        // scene change) strands any live particles from old emitters —
        // they keep ageing naturally and never respawn.
        if self.spawn_accum.len() != emitters.len() {
            self.spawn_accum.resize(emitters.len(), 0.0);
            self.per_emitter_count.resize(emitters.len(), 0);
        }

        // ── Age + integrate ──────────────────────────────────────────
        for p in self.particles.iter_mut() {
            if p.dead {
                continue;
            }
            p.age += dt;
            if p.age >= p.lifetime {
                p.dead = true;
                if let Some(c) = self.per_emitter_count.get_mut(p.emitter as usize) {
                    *c = c.saturating_sub(1);
                }
                continue;
            }
            // Buoyant acceleration: particles accelerate slightly upward
            // over their lifetime so the plume narrows as it rises.
            p.vel.z += 12.0 * dt;
            // Inherit live wind from the emitter (so gusts turn the
            // existing plume, not just future spawns).
            if let Some(e) = emitters.get(p.emitter as usize) {
                p.vel.x += e.wind.x * WIND_ACCEL * dt;
                p.vel.y += 0.0; // wind.y is a visual squash handled in shader
                // Candle-like lateral wobble: slow envelope + faster ripples.
                let ep = e.phase * std::f32::consts::TAU;
                let wob = ep + time_s * 3.15;
                let slow = time_s * 1.65 + ep;
                let curl_x =
                    (wob.sin() * 0.55 + (slow + p.phase * 2.1).sin() * 0.38) * CURL_ACCEL * dt;
                let curl_y =
                    (wob.cos() * 0.52 + (slow * 1.12 + p.phase).cos() * 0.34) * CURL_ACCEL * dt;
                p.vel.x += curl_x;
                p.vel.y += curl_y;
                // Mild horizontal damping so particles don't runaway sideways.
                let damp = (1.0 - 1.85 * dt).max(0.0);
                p.vel.x *= damp;
                p.vel.y *= damp;
            }
            p.pos += p.vel * dt;
        }

        // ── Spawn ────────────────────────────────────────────────────
        for (ei, e) in emitters.iter().enumerate() {
            // Work on a local copy of the accumulator so the rand_range
            // calls below can mutate `self.rng_state` without an
            // aliasing borrow. Write it back after the loop.
            let mut accum = self.spawn_accum[ei] + dt * SPAWN_RATE_HZ;
            while accum >= 1.0 {
                accum -= 1.0;
                if self.per_emitter_count[ei] as usize >= PER_EMITTER_CAP {
                    continue;
                }
                // Find a dead slot. Linear scan is fine at 256.
                let Some(slot) = self.particles.iter().position(|p| p.dead) else {
                    break;
                };
                // Small spawn jitter: offset sideways from the wick by a
                // couple of percent of the candle scale so overlapping
                // particles don't stack exactly.
                let jx = self.rand_range(-0.18, 0.18) * e.scale;
                let jy = self.rand_range(-0.18, 0.18) * e.scale;
                let jz = self.rand_range(0.0, 0.10) * e.scale;
                // Velocity: mostly upward with a small random cone.
                let vz = self.rand_range(0.85, 1.15) * RISE_SPEED;
                let vx =
                    self.rand_range(-0.25, 0.25) * RISE_SPEED * 0.25 + e.wind.x * WIND_ACCEL * 0.3;
                let vy = self.rand_range(-0.25, 0.25) * RISE_SPEED * 0.25;
                let lifetime = LIFETIME_MEAN + self.rand_range(-LIFETIME_JITTER, LIFETIME_JITTER);
                let phase = self.rand01() * std::f32::consts::TAU + e.phase * std::f32::consts::TAU;
                // Per-particle scale: ~half the candle scale, with a
                // little jitter. The full flame silhouette is built from
                // dozens of overlapping particles.
                let scale = e.scale * self.rand_range(0.28, 0.42);
                self.particles[slot] = Particle {
                    pos: e.wick_world + Vec3::new(jx, jy, jz),
                    vel: Vec3::new(vx, vy, vz),
                    age: 0.0,
                    lifetime,
                    scale,
                    phase,
                    emitter: ei as u16,
                    dead: false,
                };
                self.per_emitter_count[ei] += 1;
            }
            self.spawn_accum[ei] = accum;
        }

        self.live_count()
    }

    /// Total live particles. O(n) scan — call once per frame.
    pub fn live_count(&self) -> usize {
        self.particles.iter().filter(|p| !p.dead).count()
    }

    /// Write every live particle into the caller-supplied GPU-staging
    /// buffer. Emitter-derived brightness is refreshed on each write so
    /// the flame ramp tracks the current frame's value, not whatever was
    /// live at spawn time. Emits two GPU instances per live particle
    /// (cross billboards). Returns the number of instances written.
    pub fn fill_gpu_instances(
        &self,
        emitters: &[FlameEmitter],
        time_s: f32,
        out: &mut Vec<GpuFlameParticle>,
    ) -> usize {
        out.clear();
        for p in self.particles.iter() {
            if p.dead {
                continue;
            }
            let normalized_age = (p.age / p.lifetime.max(1e-3)).clamp(0.0, 1.0);
            let (base_b, emit_phase) = emitters
                .get(p.emitter as usize)
                .map(|e| (e.brightness, e.phase))
                .unwrap_or((1.0, 0.0));
            let ep = emit_phase * std::f32::consts::TAU;
            // Shared low-frequency flicker per candle (phase keeps neighbours apart).
            let flick = 1.0
                + 0.085 * (ep + time_s * 6.9).sin() * (time_s * 11.5 + emit_phase * 4.3).sin();
            let brightness = (base_b * flick).clamp(0.0, 1.38);
            for cross_slice in [0.0_f32, 1.0_f32] {
                out.push(GpuFlameParticle {
                    pos: p.pos.to_array(),
                    age: normalized_age,
                    scale: p.scale,
                    phase: p.phase,
                    brightness,
                    cross_slice,
                });
            }
        }
        out.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emitter() -> FlameEmitter {
        FlameEmitter {
            wick_world: Vec3::new(0.0, 0.0, 10.0),
            scale: 1.0,
            wind: Vec2::ZERO,
            brightness: 1.0,
            phase: 0.25,
        }
    }

    #[test]
    fn pool_fills_to_per_emitter_cap() {
        let mut sys = FlameParticleSystem::new();
        let e = [emitter()];
        // Run several seconds of 60Hz ticks; pool should saturate near the cap.
        for i in 0..180 {
            sys.step(&e, 1.0 / 60.0, i as f32 / 60.0);
        }
        let n = sys.live_count();
        assert!(
            n >= PER_EMITTER_CAP / 2 && n <= PER_EMITTER_CAP + 2,
            "expected steady-state near {PER_EMITTER_CAP}, got {n}"
        );
    }

    #[test]
    fn particles_rise() {
        let mut sys = FlameParticleSystem::new();
        let e = [emitter()];
        // Settle.
        for i in 0..120 {
            sys.step(&e, 1.0 / 60.0, i as f32 / 60.0);
        }
        // Every live particle should be above the wick; their z velocity is positive.
        for p in sys.particles.iter().filter(|p| !p.dead) {
            assert!(
                p.pos.z >= e[0].wick_world.z - 0.01,
                "particle fell below wick"
            );
        }
    }

    #[test]
    fn fill_emits_bytes_per_live_particle() {
        let mut sys = FlameParticleSystem::new();
        let e = [emitter()];
        for i in 0..120 {
            sys.step(&e, 1.0 / 60.0, i as f32 / 60.0);
        }
        let live = sys.live_count();
        let mut out = Vec::new();
        let n = sys.fill_gpu_instances(&e, 0.0, &mut out);
        assert_eq!(n, live * 2);
        assert_eq!(out.len(), live * 2);
    }
}
