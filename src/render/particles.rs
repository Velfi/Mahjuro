//! CPU-simulated particle system rendered as small quads.

use glam::{Mat4, Vec3};
use rand::RngExt;

use crate::render::draw_cmd::CameraParams;
use crate::render::raycast;
use crate::render::room_env_gltf::RoomCollisionMesh;

#[derive(Clone, Debug)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    color: [f32; 4],
    life: f32,
    max_life: f32,
    size: f32,
}

#[derive(Clone, Debug)]
struct WorldDrop {
    pos: Vec3,
    vel: Vec3,
    color: [f32; 4],
    /// Per-drop fall multiplier (world −Z); breaks lockstep recycling when many drops share wind.
    fall_speed_mul: f32,
}

/// Rain collision target (centered room env matrix + `rain_hit_*` meshes).
pub struct RainCollider<'a> {
    pub model: Mat4,
    pub meshes: &'a [RoomCollisionMesh],
}

/// Axis-aligned volume where world rain drops are born (and respawn after impact).
/// Drops integrate along world −Z.
#[derive(Clone, Copy, Debug)]
pub struct RainSpawnVolume {
    pub min: Vec3,
    pub max: Vec3,
}

impl RainSpawnVolume {
    /// Random XY inside the column; Z spread across the **upper** portion of the volume.
    ///
    /// A very thin ceiling band plus identical fall speed makes every drop hit the floor
    /// (or recycle) within a few ms of each other — reads as rhythmic "waves". Use enough
    /// vertical spread that time-to-ground differs visibly while births still read as rain
    /// from above.
    pub fn random_pos(self) -> Vec3 {
        let mut rng = rand::rng();
        let x = self.min.x + rng.random::<f32>() * (self.max.x - self.min.x).max(1e-6);
        let y = self.min.y + rng.random::<f32>() * (self.max.y - self.min.y).max(1e-6);
        let z_span = (self.max.z - self.min.z).max(1e-3);
        // Bias slightly toward the ceiling (sqrt) but cover ~upper 55% of the column.
        let t = rng.random::<f32>().sqrt();
        let z = self.max.z - t * z_span * 0.55;
        Vec3::new(x, y, z)
    }
}

pub struct ParticleSystem {
    particles: Vec<Particle>,
    world_drops: Vec<WorldDrop>,
    splashes_this_frame: u32,
}

const MAX_SPLASH_EVENTS_PER_FRAME: u32 = 48;

fn segment_hit_rain_meshes(
    origin: Vec3,
    dir_unit: Vec3,
    max_t: f32,
    model: Mat4,
    meshes: &[RoomCollisionMesh],
) -> Option<Vec3> {
    let mut best_t: Option<f32> = None;
    for mesh in meshes {
        if let Some(hit) = raycast::ray_hit_trimesh(&mesh.triangles, model, origin, dir_unit) {
            if hit.t > 1e-5 && hit.t <= max_t {
                best_t = Some(best_t.map_or(hit.t, |bt| bt.min(hit.t)));
            }
        }
    }
    best_t.map(|t| origin + dir_unit * t)
}

impl ParticleSystem {
    pub fn new() -> Self {
        Self {
            particles: Vec::new(),
            world_drops: Vec::new(),
            splashes_this_frame: 0,
        }
    }

    pub fn world_drop_count(&self) -> usize {
        self.world_drops.len()
    }

    /// Drop excess world rain when the pool cap shrinks (e.g. lower density).
    pub fn trim_world_drops_to(&mut self, max: usize) {
        if self.world_drops.len() > max {
            self.world_drops.truncate(max);
        }
    }

    /// Emit a soft puff of small particles. Used for ambient feedback.
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
                vy: angle.sin() * speed - 20.0,
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
            let speed: f32 = 120.0 + rng.random::<f32>() * 260.0;
            let size: f32 = 6.0 + rng.random::<f32>() * 14.0;
            self.particles.push(Particle {
                x,
                y,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed - 180.0,
                color,
                life: 1.0,
                max_life: lifetime,
                size,
            });
        }
    }

    /// Seed one world-space rain drop (caller manages pool size).
    pub fn spawn_world_drop(&mut self, pos: Vec3, color: [f32; 4], fall_speed_mul: f32) {
        self.world_drops.push(WorldDrop {
            pos,
            vel: Vec3::ZERO,
            color,
            fall_speed_mul,
        });
    }

    /// Radial splash at a screen position (rain impact).
    pub fn emit_splash_at(
        &mut self,
        sx: f32,
        sy: f32,
        count: usize,
        color: [f32; 4],
        lifetime: f32,
    ) {
        if self.splashes_this_frame >= MAX_SPLASH_EVENTS_PER_FRAME {
            return;
        }
        self.splashes_this_frame += 1;
        let mut rng = rand::rng();
        for _ in 0..count {
            let angle: f32 = rng.random::<f32>() * std::f32::consts::TAU;
            let speed: f32 = 40.0 + rng.random::<f32>() * 120.0;
            let size: f32 = 3.0 + rng.random::<f32>() * 7.0;
            self.particles.push(Particle {
                x: sx,
                y: sy,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed - 30.0,
                color,
                life: 1.0,
                max_life: lifetime,
                size,
            });
        }
    }

    /// Integrate world drops; raycast movement segment against rain surfaces.
    pub fn update_world(
        &mut self,
        dt: f32,
        collider: Option<RainCollider<'_>>,
        cam: &CameraParams,
        window_w: f32,
        window_h: f32,
        fall_speed: f32,
        wind: [f32; 2],
        splash_count: usize,
        splash_lifetime: f32,
        splash_color: [f32; 4],
        volume: RainSpawnVolume,
    ) {
        self.splashes_this_frame = 0;
        let mut rng = rand::rng();
        let wind = Vec3::new(wind[0], wind[1], 0.0);
        let mut hits: Vec<(usize, Vec3)> = Vec::new();
        for (i, drop) in self.world_drops.iter_mut().enumerate() {
            let m = drop.fall_speed_mul.clamp(0.72, 1.28);
            drop.vel.x = wind.x;
            drop.vel.y = wind.y;
            drop.vel.z = -fall_speed.abs() * m;
            let prev = drop.pos;
            let step = drop.vel * dt;
            drop.pos += step;
            let step_len = step.length();
            let dir = if step_len > 1e-6 {
                step / step_len
            } else {
                Vec3::new(0.0, 0.0, -1.0)
            };
            if let Some(ref c) = collider
                && !c.meshes.is_empty()
                && step_len > 1e-6
                && let Some(hit_world) =
                    segment_hit_rain_meshes(prev, dir, step_len, c.model, c.meshes)
            {
                hits.push((i, hit_world));
            } else if drop.pos.z < volume.min.z {
                drop.pos = volume.random_pos();
                drop.fall_speed_mul = 0.88 + rng.random::<f32>() * 0.24;
            }
        }
        for (i, hit_world) in hits {
            let (sx, sy) = cam.project_world_to_screen(window_w, window_h, hit_world);
            self.emit_splash_at(sx, sy, splash_count, splash_color, splash_lifetime);
            self.world_drops[i].pos = volume.random_pos();
            self.world_drops[i].fall_speed_mul = 0.88 + rng.random::<f32>() * 0.24;
        }
    }

    /// Advance simulation by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        for p in &mut self.particles {
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.vy += 80.0 * dt;
            p.life -= dt / p.max_life;
        }
        self.particles.retain(|p| p.life > 0.0);
    }

    /// Screen-space particle instances.
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

    /// World rain streaks projected to layout pixels (elongated quads along velocity).
    /// `streak_len_px` is the current tuning value so live slider edits apply to all drops.
    pub fn instances_world(
        &self,
        cam: &CameraParams,
        window_w: f32,
        window_h: f32,
        streak_len_px: f32,
    ) -> Vec<([f32; 4], [f32; 4])> {
        let mut out = Vec::with_capacity(self.world_drops.len());
        for d in &self.world_drops {
            let (sx, sy) = cam.project_world_to_screen(window_w, window_h, d.pos);
            if sx < -80.0 || sy < -80.0 || sx > window_w + 80.0 || sy > window_h + 80.0 {
                continue;
            }
            let v = d.vel;
            let v_len = v.length();
            let v_dir = if v_len > 1e-6 {
                v / v_len
            } else {
                Vec3::new(0.0, 0.0, -1.0)
            };
            // Map desired pixel streak length → world offset along fall direction.
            let (bx, by) = cam.project_world_to_screen(window_w, window_h, d.pos - v_dir * 1.0);
            let px_per_world = ((bx - sx).hypot(by - sy)).max(1e-3);
            let world_back = (streak_len_px.max(1.0) / px_per_world).min(5000.0);
            let tail = d.pos - v_dir * world_back;
            let (tx, ty) = cam.project_world_to_screen(window_w, window_h, tail);
            let dx = tx - sx;
            let dy = ty - sy;
            let len = (dx * dx + dy * dy).sqrt().max(4.0);
            let half_w = 1.2;
            let cx = (sx + tx) * 0.5;
            let cy = (sy + ty) * 0.5;
            // Axis-aligned quads: thin along X, long along Y (streaks read vertical on screen).
            let rect = [cx - half_w, cy - len * 0.5, half_w * 2.0, len];
            let alpha = d.color[3];
            out.push((rect, [d.color[0], d.color[1], d.color[2], alpha]));
        }
        out
    }

    pub fn is_active(&self) -> bool {
        !self.particles.is_empty() || !self.world_drops.is_empty()
    }
}
