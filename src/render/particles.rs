//! CPU-simulated particle system rendered as small quads.

use rand::RngExt;

#[derive(Clone, Debug)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    color: [f32; 4],
    life: f32,     // 0..1, starts at 1.0 and decreases
    max_life: f32, // total lifetime in seconds
    size: f32,
}

pub struct ParticleSystem {
    particles: Vec<Particle>,
}

impl ParticleSystem {
    pub fn new() -> Self {
        Self {
            particles: Vec::new(),
        }
    }

    /// Emit a soft puff of small particles. Used for ambient feedback.
    #[allow(dead_code)]
    pub fn emit(&mut self, x: f32, y: f32, count: usize, color: [f32; 4], lifetime: f32) {
        let mut rng = rand::rng();
        for _ in 0..count {
            let angle: f32 = rng.random::<f32>() * std::f32::consts::TAU;
            let speed: f32 = 30.0 + rng.random::<f32>() * 90.0;
            let size: f32 = 2.0 + rng.random::<f32>() * 4.0;
            self.particles.push(Particle {
                x,
                y,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed - 20.0, // slight upward bias
                color,
                life: 1.0,
                max_life: lifetime,
                size,
            });
        }
    }

    /// Emit a *real* explosion: dramatically larger particles, faster
    /// outward velocity, and a heavy upward kick so the burst reads as a
    /// firework rather than a polite puff. Used by the scoring cascade
    /// when a hand actually scores so the player feels the impact.
    pub fn explode(&mut self, x: f32, y: f32, count: usize, color: [f32; 4], lifetime: f32) {
        let mut rng = rand::rng();
        for _ in 0..count {
            let angle: f32 = rng.random::<f32>() * std::f32::consts::TAU;
            // ~3× the soft-puff outward speed.
            let speed: f32 = 120.0 + rng.random::<f32>() * 260.0;
            // Big chunky shards so the explosion is visible from across
            // the screen, not just up close to the source.
            let size: f32 = 6.0 + rng.random::<f32>() * 14.0;
            self.particles.push(Particle {
                x,
                y,
                vx: angle.cos() * speed,
                // Strong upward bias — fireworks-style.
                vy: angle.sin() * speed - 180.0,
                color,
                life: 1.0,
                max_life: lifetime,
                size,
            });
        }
    }

    /// Advance simulation by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        for p in &mut self.particles {
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.vy += 80.0 * dt; // gravity
            p.life -= dt / p.max_life;
        }
        self.particles.retain(|p| p.life > 0.0);
    }

    /// Generate GPU instances for rendering. Returns `(x, y, w, h, r, g, b, a)` tuples
    /// compatible with GpuInstance format.
    pub fn instances(&self) -> Vec<([f32; 4], [f32; 4])> {
        self.particles
            .iter()
            .map(|p| {
                let alpha = p.color[3] * p.life.max(0.0);
                (
                    [p.x - p.size * 0.5, p.y - p.size * 0.5, p.size, p.size],
                    [p.color[0], p.color[1], p.color[2], alpha],
                )
            })
            .collect()
    }

    pub fn is_active(&self) -> bool {
        !self.particles.is_empty()
    }
}
