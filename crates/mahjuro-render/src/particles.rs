//! CPU-simulated particle system rendered as small quads.

use glam::{Mat4, Vec3};
use rand::RngExt;

use crate::draw_cmd::{CameraParams, ScreenProjector};
use crate::raycast;
use crate::room_env_gltf::RoomCollisionMesh;
use crate::scene_light_sample::{
    RainVolumetricLit, SceneLightSampleCtx, shade_dielectric_rgb_at_world,
};

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
    /// Per-drop fall multiplier (world −Z); breaks lockstep recycling when many drops share wind.
    fall_speed_mul: f32,
}

/// Rain collision target (centered room env matrix + merged `rain_hit_*` soup).
pub struct RainCollider<'a> {
    pub model: Mat4,
    pub inv_model: Mat4,
    pub mesh: &'a RoomCollisionMesh,
}

/// Axis-aligned volume where world rain drops are born (and respawn after impact).
/// Drops integrate along world −Z.
#[derive(Clone, Copy, Debug)]
pub struct RainSpawnVolume {
    pub min: Vec3,
    pub max: Vec3,
}

/// Minimum spawn/respawn weight at the far end of the volume (faint background rain).
const RAIN_SPAWN_FAR_FLOOR: f32 = 0.12;
const RAIN_SPAWN_BIAS_MAX_TRIES: u32 = 16;

fn rain_view_forward(cam: &CameraParams) -> Vec3 {
    let eye = Vec3::from_array(cam.eye);
    let target = Vec3::from_array(cam.target);
    (target - eye).normalize_or_zero()
}

fn rain_view_right(cam: &CameraParams) -> Vec3 {
    let forward = rain_view_forward(cam);
    let up = Vec3::from_array(cam.up);
    forward.cross(up).normalize_or_zero()
}

fn rain_view_depth(cam: &CameraParams, pos: Vec3) -> f32 {
    let eye = Vec3::from_array(cam.eye);
    (pos - eye).dot(rain_view_forward(cam))
}

fn rain_view_lateral(cam: &CameraParams, pos: Vec3) -> f32 {
    let eye = Vec3::from_array(cam.eye);
    (pos - eye).dot(rain_view_right(cam)).abs()
}

impl RainSpawnVolume {
    /// Horizontal half-width of the view frustum at `depth` along view forward (world units).
    pub fn frustum_lateral_half_at(cam: &CameraParams, depth: f32, aspect: f32) -> f32 {
        depth.max(0.0) * (cam.fovy_deg.to_radians() * 0.5).tan() * aspect.max(1e-6)
    }

    fn random_pos_z(self) -> f32 {
        let mut rng = rand::rng();
        let z_span = (self.max.z - self.min.z).max(1e-3);
        let t = rng.random::<f32>().sqrt();
        self.max.z - t * z_span * 0.55
    }

    /// Random position inside the spawn column ∩ camera view frustum (uniform in frustum).
    pub fn random_pos_in_frustum(self, cam: &CameraParams, aspect: f32) -> Vec3 {
        let (d_min, d_max) = self.frustum_depth_range(cam);
        let eye = Vec3::from_array(cam.eye);
        let forward = rain_view_forward(cam);
        let right = rain_view_right(cam);
        let mut rng = rand::rng();
        for _ in 0..RAIN_SPAWN_BIAS_MAX_TRIES {
            let depth = d_min + rng.random::<f32>() * (d_max - d_min).max(1e-3);
            let lat_half = Self::frustum_lateral_half_at(cam, depth, aspect);
            let lateral = (rng.random::<f32>() * 2.0 - 1.0) * lat_half;
            let mut pos = eye + forward * depth + right * lateral;
            pos.z = self.random_pos_z();
            if self.contains(pos) {
                return pos;
            }
        }
        self.random_pos()
    }

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
        Vec3::new(x, y, self.random_pos_z())
    }

    /// Frustum spawn biased toward the camera in the view plane (depth + lateral).
    /// `near_bias` ≤ 0 skips bias (uniform in frustum).
    pub fn random_pos_near_camera(
        self,
        cam: &CameraParams,
        aspect: f32,
        near_bias: f32,
    ) -> Vec3 {
        if near_bias <= 0.0 {
            return self.random_pos_in_frustum(cam, aspect);
        }
        let (d_min, d_max) = self.frustum_depth_range(cam);
        let mut rng = rand::rng();
        for _ in 0..RAIN_SPAWN_BIAS_MAX_TRIES {
            let pos = self.random_pos_in_frustum(cam, aspect);
            if rng.random::<f32>()
                <= Self::spawn_acceptance(cam, pos, aspect, d_min, d_max, near_bias)
            {
                return pos;
            }
        }
        self.random_pos_in_frustum(cam, aspect)
    }

    #[inline]
    pub fn contains(self, p: Vec3) -> bool {
        p.x >= self.min.x
            && p.x <= self.max.x
            && p.y >= self.min.y
            && p.y <= self.max.y
            && p.z >= self.min.z
            && p.z <= self.max.z
    }

    /// Spawn acceptance weight at `pos` (1 = near camera, [`RAIN_SPAWN_FAR_FLOOR`] = far).
    pub fn spawn_weight_at(
        self,
        cam: &CameraParams,
        pos: Vec3,
        aspect: f32,
        near_bias: f32,
    ) -> f32 {
        if near_bias <= 0.0 {
            return if self.in_view_frustum(cam, pos, aspect) {
                1.0
            } else {
                0.0
            };
        }
        let (d_min, d_max) = self.frustum_depth_range(cam);
        Self::spawn_acceptance(cam, pos, aspect, d_min, d_max, near_bias)
    }

    /// View-depth span used for frustum rain: near = eye plane, far = farthest in-volume point
    /// in front of the camera.
    pub fn frustum_depth_range(self, cam: &CameraParams) -> (f32, f32) {
        self.view_depth_range(cam)
    }

    /// True when `pos` lies inside the horizontal FOV wedge out to the volume far depth.
    pub fn in_view_frustum(self, cam: &CameraParams, pos: Vec3, aspect: f32) -> bool {
        let depth = rain_view_depth(cam, pos);
        if depth < 0.0 {
            return false;
        }
        let (_, d_max) = self.frustum_depth_range(cam);
        if depth > d_max {
            return false;
        }
        let lat_half = Self::frustum_lateral_half_at(cam, depth, aspect);
        rain_view_lateral(cam, pos) <= lat_half
    }

    /// View-depth span of this volume in front of `cam.eye` along view forward.
    ///
    /// Near is always the eye plane (`0`); far is the farthest in-frustum corner. Volume
    /// extent behind the camera is ignored so spawn bias does not favor rain behind the player.
    pub fn view_depth_range(self, cam: &CameraParams) -> (f32, f32) {
        let mut d_max = f32::NEG_INFINITY;
        for xi in [self.min.x, self.max.x] {
            for yi in [self.min.y, self.max.y] {
                for zi in [self.min.z, self.max.z] {
                    let d = rain_view_depth(cam, Vec3::new(xi, yi, zi));
                    if d >= 0.0 {
                        d_max = d_max.max(d);
                    }
                }
            }
        }
        if d_max.is_finite() && d_max > 0.0 {
            (0.0, d_max)
        } else {
            (0.0, 1.0)
        }
    }

    /// Half-width of this volume in front of the camera, measured along view right.
    pub fn view_lateral_half(self, cam: &CameraParams) -> f32 {
        let mut half = 0.0f32;
        for xi in [self.min.x, self.max.x] {
            for yi in [self.min.y, self.max.y] {
                for zi in [self.min.z, self.max.z] {
                    let p = Vec3::new(xi, yi, zi);
                    if rain_view_depth(cam, p) >= 0.0 {
                        half = half.max(rain_view_lateral(cam, p));
                    }
                }
            }
        }
        half.max(1.0)
    }

    fn spawn_acceptance(
        cam: &CameraParams,
        pos: Vec3,
        aspect: f32,
        d_min: f32,
        d_max: f32,
        near_bias: f32,
    ) -> f32 {
        let depth = rain_view_depth(cam, pos);
        if depth < 0.0 {
            return 0.0;
        }
        let lat_half = Self::frustum_lateral_half_at(cam, depth, aspect);
        let lateral = rain_view_lateral(cam, pos);
        if lateral > lat_half {
            return 0.0;
        }
        let depth_span = (d_max - d_min).max(1.0);
        let t_depth = ((depth - d_min) / depth_span).clamp(0.0, 1.0);
        let t_lat = if lat_half > 1e-3 {
            (lateral / lat_half).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let near = (1.0 - t_depth) * (1.0 - t_lat);
        (RAIN_SPAWN_FAR_FLOOR + (1.0 - RAIN_SPAWN_FAR_FLOOR) * near.powf(near_bias.max(0.0)))
            .clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod rain_spawn_bias_tests {
    use super::*;

    fn axis_cam() -> CameraParams {
        CameraParams {
            eye: [0.0, -200.0, 100.0],
            target: [0.0, 200.0, 100.0],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 50.0,
            clip_near: None,
            clip_far: None,
        }
    }

    #[test]
    fn spawn_bias_peaks_on_camera_axis_in_front() {
        let cam = axis_cam();
        let vol = RainSpawnVolume {
            min: Vec3::new(-400.0, -100.0, 80.0),
            max: Vec3::new(400.0, 500.0, 200.0),
        };
        let bias = 2.5;
        let eye = Vec3::from_array(cam.eye);
        let forward = rain_view_forward(&cam);
        let right = rain_view_right(&cam);
        let on_axis_near = eye + forward * 50.0;
        let on_axis_far = eye + forward * 400.0;
        let off_axis = on_axis_near + right * 250.0;

        let aspect = 16.0 / 9.0;
        let w_near = vol.spawn_weight_at(&cam, on_axis_near, aspect, bias);
        let w_far = vol.spawn_weight_at(&cam, on_axis_far, aspect, bias);
        let w_lat = vol.spawn_weight_at(&cam, off_axis, aspect, bias);
        let w_behind = vol.spawn_weight_at(&cam, eye - forward * 80.0, aspect, bias);

        assert!(w_near > w_far, "near axis should beat far: {w_near} vs {w_far}");
        assert!(w_near > w_lat, "center should beat lateral: {w_near} vs {w_lat}");
        assert!(w_behind < 1e-5, "behind camera should be rejected, got {w_behind}");
    }
}

pub struct ParticleSystem {
    particles: Vec<Particle>,
    world_drops: Vec<WorldDrop>,
    splashes_this_frame: u32,
    rain_hits_scratch: Vec<(usize, Vec3, Vec3)>,
}

const MAX_SPLASH_EVENTS_PER_FRAME: u32 = 48;
const SPLASH_SIZE_REF_DISTANCE: f32 = 350.0;
const SPLASH_SIZE_MIN_MUL: f32 = 0.45;
const SPLASH_SIZE_MAX_MUL: f32 = 1.9;

#[inline]
fn rain_on_screen(proj: ScreenProjector, pos: Vec3, margin: f32) -> bool {
    let (sx, sy) = proj.project(pos);
    sx >= -margin
        && sy >= -margin
        && sx <= proj.window_w() + margin
        && sy <= proj.window_h() + margin
}

fn segment_hit_rain_mesh(
    origin: Vec3,
    dir_unit: Vec3,
    max_t: f32,
    model: Mat4,
    inv: Mat4,
    mesh: &RoomCollisionMesh,
) -> Option<(Vec3, Vec3)> {
    let lo = inv.transform_point3(origin);
    let ld = inv.transform_vector3(dir_unit);
    if !raycast::ray_segment_hits_local_aabb(lo, ld, mesh.local_min, mesh.local_max, max_t) {
        return None;
    }
    raycast::ray_hit_trimesh_inv(&mesh.triangles, model, inv, origin, dir_unit)
        .filter(|hit| hit.t > 1e-5 && hit.t <= max_t)
        .map(|hit| (origin + dir_unit * hit.t, hit.normal))
}

#[inline]
fn splash_size_mul_for_distance(cam: &CameraParams, hit_world: Vec3) -> f32 {
    let eye = Vec3::from_array(cam.eye);
    let dist = (hit_world - eye).length().max(1.0);
    (SPLASH_SIZE_REF_DISTANCE / dist).clamp(SPLASH_SIZE_MIN_MUL, SPLASH_SIZE_MAX_MUL)
}

impl ParticleSystem {
    pub fn new() -> Self {
        Self {
            particles: Vec::new(),
            world_drops: Vec::new(),
            splashes_this_frame: 0,
            rain_hits_scratch: Vec::new(),
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
    pub fn spawn_world_drop(&mut self, pos: Vec3, fall_speed_mul: f32) {
        self.world_drops.push(WorldDrop {
            pos,
            vel: Vec3::ZERO,
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
        size_mul: f32,
    ) {
        if self.splashes_this_frame >= MAX_SPLASH_EVENTS_PER_FRAME {
            return;
        }
        self.splashes_this_frame += 1;
        let size_mul = size_mul.clamp(SPLASH_SIZE_MIN_MUL, SPLASH_SIZE_MAX_MUL);
        let mut rng = rand::rng();
        for _ in 0..count {
            let angle: f32 = rng.random::<f32>() * std::f32::consts::TAU;
            let speed: f32 = 40.0 + rng.random::<f32>() * 120.0;
            let size: f32 = (3.0 + rng.random::<f32>() * 7.0) * size_mul;
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
        spawn_near_bias: f32,
        lighting: Option<&SceneLightSampleCtx<'_>>,
    ) {
        self.splashes_this_frame = 0;
        let mut rng = rand::rng();
        let aspect = window_w / window_h.max(1.0);
        let wind = Vec3::new(wind[0], wind[1], 0.0);
        let proj = cam.screen_projector(window_w, window_h);
        self.rain_hits_scratch.clear();
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
            let needs_collision = rain_on_screen(proj, prev, 80.0) || rain_on_screen(proj, drop.pos, 80.0);
            if let Some(ref c) = collider
                && needs_collision
                && step_len > 1e-6
                && let Some((hit_world, hit_normal)) = segment_hit_rain_mesh(
                    prev,
                    dir,
                    step_len,
                    c.model,
                    c.inv_model,
                    c.mesh,
                )
            {
                self.rain_hits_scratch.push((i, hit_world, hit_normal));
            } else if drop.pos.z < volume.min.z {
                drop.pos = volume.random_pos_near_camera(cam, aspect, spawn_near_bias);
                drop.fall_speed_mul = 0.88 + rng.random::<f32>() * 0.24;
            }
        }
        let hits = std::mem::take(&mut self.rain_hits_scratch);
        for (i, hit_world, hit_normal) in hits {
            let (sx, sy) = cam.project_world_to_screen(window_w, window_h, hit_world);
            let splash_rgb = if let Some(ctx) = lighting {
                shade_dielectric_rgb_at_world(
                    hit_world,
                    hit_normal,
                    [
                        splash_color[0],
                        splash_color[1],
                        splash_color[2],
                    ],
                    ctx,
                )
            } else {
                [
                    splash_color[0],
                    splash_color[1],
                    splash_color[2],
                ]
            };
            let lit_splash = [
                splash_rgb[0],
                splash_rgb[1],
                splash_rgb[2],
                splash_color[3],
            ];
            let splash_size_mul = splash_size_mul_for_distance(cam, hit_world);
            self.emit_splash_at(
                sx,
                sy,
                splash_count,
                lit_splash,
                splash_lifetime,
                splash_size_mul,
            );
            self.world_drops[i].pos = volume.random_pos_near_camera(cam, aspect, spawn_near_bias);
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
    pub fn fill_world_instances(
        &self,
        out: &mut Vec<([f32; 4], [f32; 4])>,
        proj: ScreenProjector,
        cam: &CameraParams,
        streak_len_px: f32,
        drop_color: [f32; 4],
        lit: Option<&RainVolumetricLit>,
    ) {
        let base_rgb = [drop_color[0], drop_color[1], drop_color[2]];
        let margin = 80.0;
        let w = proj.window_w();
        let h = proj.window_h();
        out.reserve(self.world_drops.len());
        for d in &self.world_drops {
            let (sx, sy) = proj.project(d.pos);
            if sx < -margin || sy < -margin || sx > w + margin || sy > h + margin {
                continue;
            }
            let rgb = lit
                .map(|l| l.sample_at(d.pos, cam))
                .unwrap_or(base_rgb);
            let v = d.vel;
            let v_len = v.length();
            let v_dir = if v_len > 1e-6 {
                v / v_len
            } else {
                Vec3::new(0.0, 0.0, -1.0)
            };
            // Map desired pixel streak length → world offset along fall direction.
            let (bx, by) = proj.project(d.pos - v_dir * 1.0);
            let px_per_world = ((bx - sx).hypot(by - sy)).max(1e-3);
            let world_back = (streak_len_px.max(1.0) / px_per_world).min(5000.0);
            let tail = d.pos - v_dir * world_back;
            let (tx, ty) = proj.project(tail);
            let dx = tx - sx;
            let dy = ty - sy;
            let len = (dx * dx + dy * dy).sqrt().max(4.0);
            let half_w = 1.2;
            let cx = (sx + tx) * 0.5;
            let cy = (sy + ty) * 0.5;
            // Axis-aligned quads: thin along X, long along Y (streaks read vertical on screen).
            let rect = [cx - half_w, cy - len * 0.5, half_w * 2.0, len];
            let alpha = drop_color[3];
            out.push((rect, [rgb[0], rgb[1], rgb[2], alpha]));
        }
    }

    pub fn is_active(&self) -> bool {
        !self.particles.is_empty() || !self.world_drops.is_empty()
    }
}
