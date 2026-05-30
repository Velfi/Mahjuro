//! World-space rain volume for exterior scenes (main menu).
//!
//! Spawns drops in a thin band under the spawn ceiling of a padded `rain_hit_*` AABB, integrates in world −Z with wind,
//! raycasts against `rain_hit_*` collision shells, and emits world-space splashes.

use std::cell::RefCell;

use glam::Vec3;
use rand::RngExt;

use crate::draw_cmd::{CameraParams, UiFrame};
use crate::main_menu_glb;
use crate::particles::{ParticleSystem, RainCollider, RainSpawnVolume, RainSplashUpdate};
use crate::rain_tuning::RainTuning;
use crate::room_env_gltf::RoomCollisionMesh;
use crate::room_glb;
use crate::scene_light_sample::{RainVolumetricLit, SceneLightSampleCtx};
use crate::wgpu_renderer::GpuInstance;

pub struct RainField {
    particles: ParticleSystem,
    /// Fractional spawn credits (mean rate = `spawn_rate` when threshold mean is 1).
    spawn_accum: f32,
    /// Random credits required for the next drop (uniform ~[0.35, 1.65], mean 1).
    /// Avoids fixed 1.0 steps that align many births on the same frames ("waves").
    spawn_next_threshold: f32,
    quad_scratch: RefCell<Vec<GpuInstance>>,
    instance_scratch: RefCell<Vec<([f32; 4], [f32; 4])>>,
}

impl Default for RainField {
    fn default() -> Self {
        Self::new()
    }
}

impl RainField {
    fn random_spawn_threshold() -> f32 {
        let mut rng = rand::rng();
        0.35 + rng.random::<f32>() * 1.3
    }

    pub fn new() -> Self {
        Self {
            particles: ParticleSystem::new(),
            spawn_accum: 0.0,
            spawn_next_threshold: Self::random_spawn_threshold(),
            quad_scratch: RefCell::new(Vec::new()),
            instance_scratch: RefCell::new(Vec::new()),
        }
    }

    pub fn update(
        &mut self,
        dt: f32,
        tuning: &RainTuning,
        cam: &CameraParams,
        window_w: f32,
        window_h: f32,
        env_scale: f32,
        rain_mesh: Option<&RoomCollisionMesh>,
        lighting: Option<&SceneLightSampleCtx<'_>>,
    ) {
        let speed = tuning.speed_mul.max(0.0);
        let volume = main_menu_rain_spawn_volume(env_scale, window_h, tuning);
        let density = tuning.field.density.max(0.0);
        let pool = (tuning.field.pool_size * density).round().max(0.0) as usize;
        let spawn_rate = tuning.field.spawn_rate.max(0.0) * density * speed.max(0.25);
        self.particles.trim_world_drops_to(pool);
        if pool == 0 {
            self.spawn_accum = 0.0;
        } else {
            self.spawn_accum += spawn_rate * dt;
            let mut rng = rand::rng();
            while self.spawn_accum >= self.spawn_next_threshold
                && self.particles.world_drop_count() < pool
            {
                self.spawn_accum -= self.spawn_next_threshold;
                self.spawn_next_threshold = 0.35 + rng.random::<f32>() * 1.3;
                let fall_mul = 0.88 + rng.random::<f32>() * 0.24;
                let aspect = window_w / window_h.max(1.0);
                self.particles.spawn_world_drop(
                    volume.random_pos_near_camera(cam, aspect, tuning.field.spawn_near_bias),
                    fall_mul,
                );
            }
        }

        let model = main_menu_glb::main_menu_rain_env_model_matrix(window_h, env_scale);
        let collider = model.zip(rain_mesh).map(|(m, mesh)| RainCollider {
            model: m,
            inv_model: m.inverse(),
            mesh,
        });

        self.particles.update_world(
            dt,
            collider,
            cam,
            window_w,
            window_h,
            tuning.field_fall_speed_world(),
            [tuning.field.wind_x * speed, tuning.field.wind_y * speed],
            tuning.field.splash_count.round().max(1.0) as usize,
            tuning.field.splash_lifetime,
            tuning.field.drop_color,
            volume,
            tuning.field.spawn_near_bias,
            lighting,
        );
        self.particles.update(
            dt,
            Some(RainSplashUpdate {
                cam,
                window_w,
                window_h,
                fall_speed: tuning.field_fall_speed_world(),
            }),
        );
    }

    pub fn push_quads(
        &self,
        frame: &mut UiFrame,
        cam: &CameraParams,
        window_w: f32,
        window_h: f32,
        streak_len_px: f32,
        drop_color: [f32; 4],
        lit: Option<RainVolumetricLit>,
    ) {
        if !self.particles.is_active() {
            return;
        }
        let proj = cam.screen_projector(window_w, window_h);
        let mut instance_scratch = self.instance_scratch.borrow_mut();
        instance_scratch.clear();
        self.particles.fill_world_instances(
            &mut instance_scratch,
            proj,
            cam,
            streak_len_px,
            drop_color,
            lit.as_ref(),
        );
        let mut quad_scratch = self.quad_scratch.borrow_mut();
        quad_scratch.clear();
        quad_scratch.reserve(instance_scratch.len() + 64);
        for (rect, color) in instance_scratch
            .iter()
            .copied()
            .chain(self.particles.instances())
        {
            quad_scratch.push(GpuInstance {
                rect,
                color,
                user: 0,
            });
        }
        if !quad_scratch.is_empty() {
            frame.quads(std::mem::take(&mut *quad_scratch));
        }
    }
}

/// Padded spawn column over all main-menu `rain_hit_*` shells (falls back to full room AABB).
pub fn main_menu_rain_spawn_volume(
    env_scale: f32,
    window_h: f32,
    tuning: &RainTuning,
) -> RainSpawnVolume {
    rain_spawn_volume(env_scale, window_h, tuning)
}

fn rain_spawn_volume(env_scale: f32, window_h: f32, tuning: &RainTuning) -> RainSpawnVolume {
    let (min, max) = main_menu_glb::main_menu_rain_hit_spawn_aabb(window_h, env_scale)
        .or_else(|| room_spawn_aabb(env_scale, window_h))
        .unwrap_or((
            [-500.0, -350.0, window_h * 0.2],
            [500.0, 350.0, window_h * 0.85],
        ));
    let pad_xy = tuning.field.volume_pad_xy.max(0.0);
    let ext_x = (max[0] - min[0]).max(1.0);
    let ext_y = (max[1] - min[1]).max(1.0);
    let ext_z = (max[2] - min[2]).max(1.0);
    let pad_x = ext_x * pad_xy;
    let pad_y = ext_y * pad_xy;
    let top_extra = ext_z * tuning.field.volume_top_mul.max(0.1);
    RainSpawnVolume {
        min: Vec3::new(min[0] - pad_x, min[1] - pad_y, min[2] - ext_z * 0.05),
        max: Vec3::new(max[0] + pad_x, max[1] + pad_y, max[2] + top_extra),
    }
}

/// Room AABB in centered world space (same basis as rain collision + GPU room).
fn room_spawn_aabb(env_scale: f32, window_h: f32) -> Option<([f32; 3], [f32; 3])> {
    main_menu_glb::with_main_menu_glb_cpu(|opt| {
        let cpu = opt?;
        let corners = room_glb::room_world_bounds_corners_centered(window_h, env_scale, cpu);
        if corners.is_empty() {
            return None;
        }
        let mut min = corners[0].to_array();
        let mut max = min;
        for c in corners.iter().skip(1) {
            let a = c.to_array();
            for i in 0..3 {
                min[i] = min[i].min(a[i]);
                max[i] = max[i].max(a[i]);
            }
        }
        Some((min, max))
    })
}
