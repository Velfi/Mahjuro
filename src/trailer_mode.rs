//! Debug **Trailer Mode** — per-scene capture-reel hooks (Debug → Trigger Trailer Mode).

use std::time::Instant;

use crate::render::draw_cmd::CameraParams;
use crate::render::hallway_glb::HallwayDistortionDebugSnapshot;
use crate::scenes::object3d_inspect::lerp_camera;
#[cfg(debug_menu_enabled)]
use crate::game::run::RunState;
#[cfg(debug_menu_enabled)]
use crate::render::main_menu_glb::{self, main_menu_env_height_scale};
#[cfg(debug_menu_enabled)]
use crate::scenes::Scene;

const HALLWAY_DURATION_SECS: f32 = 5.0;
const MAIN_MENU_DURATION_SECS: f32 = 7.0;

const MAX_VARIATION_INDEX: u32 = 55;
const RAMP_EXP_K: f32 = 5.0;

const DISTORTION_GLOBAL_END: f32 = 1.85;
const DISTORTION_PULSE_END: f32 = 2.25;
const DISTORTION_DRIFT_END: f32 = 1.75;
const DISTORTION_RIPPLE_TRAVEL_END: f32 = 1.4;

/// Active trailer-mode sequence for the current scene.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum TrailerMode {
    Hallway(HallwayTrailer),
    MainMenu(MainMenuTrailer),
}

impl TrailerMode {
    /// Start trailer mode when the active scene supports it.
    #[cfg(debug_menu_enabled)]
    pub fn try_start(
        scene: &Scene,
        run: &RunState,
        window_w: f32,
        window_h: f32,
        room_gltf_height_scale: f32,
    ) -> Option<Self> {
        match scene {
            Scene::Hallway(_) => Some(Self::Hallway(HallwayTrailer::start(
                run.chronicle.seed,
                run.run_number,
            ))),
            Scene::MainMenu(_) => {
                let env_h = main_menu_env_height_scale(room_gltf_height_scale);
                MainMenuTrailer::start(window_w, window_h, env_h).map(Self::MainMenu)
            }
            _ => None,
        }
    }

    pub fn finished_at(&self, now: Instant) -> bool {
        match self {
            Self::Hallway(t) => t.finished_at(now),
            Self::MainMenu(t) => t.finished_at(now),
        }
    }

    pub fn hallway_snapshot_at(&self, now: Instant) -> Option<HallwayDistortionDebugSnapshot> {
        match self {
            Self::Hallway(t) => t.snapshot_at(now),
            Self::MainMenu(_) => None,
        }
    }

    pub fn main_menu_camera_at(&self, now: Instant, window_h: f32) -> Option<CameraParams> {
        match self {
            Self::MainMenu(t) => t.camera_at(now, window_h),
            Self::Hallway(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct HallwayTrailer {
    started_at: Instant,
    base_run_seed: u64,
    run_number: u32,
}

impl HallwayTrailer {
    #[cfg(debug_menu_enabled)]
    fn start(base_run_seed: u64, run_number: u32) -> Self {
        Self {
            started_at: Instant::now(),
            base_run_seed,
            run_number: run_number.max(1),
        }
    }

    fn snapshot_at(&self, now: Instant) -> Option<HallwayDistortionDebugSnapshot> {
        let elapsed = now.saturating_duration_since(self.started_at).as_secs_f32();
        if elapsed >= HALLWAY_DURATION_SECS {
            return None;
        }
        let index = hallway_variation_index(elapsed);
        let (run_seed, wing) = hallway_variation_params(self.base_run_seed, index);
        Some(hallway_trailer_snapshot(
            run_seed,
            self.run_number,
            wing,
            elapsed,
        ))
    }

    fn finished_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started_at).as_secs_f32() >= HALLWAY_DURATION_SECS
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MainMenuTrailer {
    started_at: Instant,
    start_cam: CameraParams,
    end_cam: CameraParams,
}

impl MainMenuTrailer {
    #[cfg(debug_menu_enabled)]
    fn start(window_w: f32, window_h: f32, env_h: f32) -> Option<Self> {
        if !main_menu_glb::main_menu_room_draw_ready() {
            return None;
        }
        let end_cam = main_menu_glb::main_menu_camera_base(window_w, window_h, env_h);
        let start_cam = main_menu_glb::main_menu_moon_trailer_start_camera(
            window_w, window_h, env_h, &end_cam,
        )?;
        Some(Self {
            started_at: Instant::now(),
            start_cam,
            end_cam,
        })
    }

    fn camera_at(&self, now: Instant, window_h: f32) -> Option<CameraParams> {
        let elapsed = now.saturating_duration_since(self.started_at).as_secs_f32();
        if elapsed >= MAIN_MENU_DURATION_SECS {
            return None;
        }
        let t = (elapsed / MAIN_MENU_DURATION_SECS).clamp(0.0, 1.0);
        let t = smoothstep(t);
        Some(lerp_camera(&self.start_cam, &self.end_cam, t, window_h))
    }

    fn finished_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started_at).as_secs_f32() >= MAIN_MENU_DURATION_SECS
    }
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn hallway_ramp_progress(elapsed_secs: f32) -> f32 {
    let t = (elapsed_secs / HALLWAY_DURATION_SECS).clamp(0.0, 1.0);
    ((RAMP_EXP_K * t).exp() - 1.0) / (RAMP_EXP_K.exp() - 1.0)
}

#[inline]
fn hallway_ramp_lerp(elapsed_secs: f32, from: f32, to: f32) -> f32 {
    from + (to - from) * hallway_ramp_progress(elapsed_secs)
}

#[inline]
fn hallway_variation_index(elapsed_secs: f32) -> u32 {
    (hallway_ramp_progress(elapsed_secs) * MAX_VARIATION_INDEX as f32)
        .floor()
        .clamp(0.0, MAX_VARIATION_INDEX as f32) as u32
}

#[inline]
fn hallway_variation_params(base_seed: u64, index: u32) -> (u64, u32) {
    let i = index as u64;
    let run_seed = base_seed
        ^ i.wrapping_mul(0x9E37_79B1_85EB_CA87)
        ^ i.rotate_left(17).wrapping_mul(0x517c_c1b7_2722_0a95);
    let wing = 1 + (index % 7);
    (run_seed, wing)
}

#[inline]
fn hallway_trailer_snapshot(
    run_seed: u64,
    run_number: u32,
    wing: u32,
    elapsed_secs: f32,
) -> HallwayDistortionDebugSnapshot {
    HallwayDistortionDebugSnapshot {
        chamber_mode: 0,
        run_seed,
        run_number,
        wing: wing.max(1),
        global_mul: hallway_ramp_lerp(elapsed_secs, 1.0, DISTORTION_GLOBAL_END),
        breathe_mul: 1.0,
        ceiling_mul: 1.0,
        stretch_mul: 1.0,
        twist_mul: 1.0,
        pulse_mul: hallway_ramp_lerp(elapsed_secs, 1.0, DISTORTION_PULSE_END),
        drift_mul: hallway_ramp_lerp(elapsed_secs, 1.0, DISTORTION_DRIFT_END),
        ripple_mul: 1.0,
        balloon_mul: 1.0,
        wall_tint: 0,
        ripple_waves_mul: 1.0,
        ripple_travel_mul: hallway_ramp_lerp(elapsed_secs, 1.0, DISTORTION_RIPPLE_TRAVEL_END),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hallway_variation_index_ramps_up() {
        let i0 = hallway_variation_index(0.0);
        let i_mid = hallway_variation_index(2.5);
        let i_end = hallway_variation_index(5.0);
        assert_eq!(i0, 0);
        assert!(i_mid > i0);
        assert!(i_end > i_mid);
        assert_eq!(i_end, MAX_VARIATION_INDEX);
    }

    #[test]
    fn hallway_distortion_intensity_ramps_up() {
        let start = hallway_trailer_snapshot(0, 1, 1, 0.0);
        let mid = hallway_trailer_snapshot(0, 1, 1, 2.5);
        let end = hallway_trailer_snapshot(0, 1, 1, 4.99);
        assert_eq!(start.global_mul, 1.0);
        assert!(mid.global_mul > start.global_mul);
        assert!(end.global_mul > mid.global_mul);
        assert!((end.global_mul - DISTORTION_GLOBAL_END).abs() < 0.02);
    }

    #[test]
    fn main_menu_trailer_progress_is_monotonic() {
        let start = CameraParams {
            eye: [0.0, -1.0, 0.5],
            target: [0.0, 0.0, 0.5],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 50.0,
            clip_near: None,
            clip_far: None,
        };
        let end = CameraParams {
            eye: [0.0, -5.0, 1.0],
            target: [0.0, 0.0, 0.2],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 55.0,
            clip_near: None,
            clip_far: None,
        };
        let trailer = MainMenuTrailer {
            started_at: Instant::now(),
            start_cam: start,
            end_cam: end,
        };
        let t0 = trailer.camera_at(trailer.started_at, 720.0).unwrap();
        let t_mid = trailer
            .camera_at(
                trailer.started_at + std::time::Duration::from_secs_f32(3.5),
                720.0,
            )
            .unwrap();
        assert!((t0.eye[1] - start.eye[1]).abs() < 1e-4);
        assert!(t_mid.eye[1] < t0.eye[1]);
    }
}
