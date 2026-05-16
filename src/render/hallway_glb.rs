//! [`hallway.glb`](../../../assets/3d/hallway.glb) — pick-blind hallway room.
//!
//! Marker object names (Blender → glTF):
//! - `btn_play_round` — commit to the upcoming blind (Small/Big).
//! - `btn_play_boss` — optional boss-only play control (falls back to `btn_play_round` if absent).
//! - `btn_skip_round` — skip for the tribute tag when allowed (non-boss).
//! - Perspective cameras named `default` and `boss` select pick-blind framing (see
//!   [`hallway_camera_pick_blind`]); legacy single-camera files still use the first embedded camera.
//!
//! Decodes through [`crate::render::room_env_gltf`]; decoded layout matches [`crate::render::shop_glb::RoomGlbCpu`]
//! for the shared GPU path (`shop_glb.wgsl` / embedded lights).

use std::sync::RwLock;

use glam::Vec3;

use crate::core::rules::BlindKind;
use crate::render::draw_cmd::CameraParams;
use crate::render::room_env_gltf::{
    RoomEnvWalkHooks, RoomMeshPolicy, glb_punctual_range_world_upload,
};
use crate::render::shop_glb::{self, RoomGlbCpu, ShopEnvLightingTune, load_room_glb_from_bytes};
use crate::render::wgpu_renderer::{MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS, PointLight, SpotLight};
use crate::render::world_space::surface_anchor_from_world_xyz;

/// glTF node names for pick-blind actions (must match Blender objects).
pub const BTN_PLAY_ROUND: &str = "btn_play_round";
pub const BTN_PLAY_BOSS: &str = "btn_play_boss";
pub const BTN_SKIP_ROUND: &str = "btn_skip_round";

#[inline]
fn pick_blind_embedded_camera_doc(
    cpu: &RoomGlbCpu,
    boss_blind: bool,
) -> Option<shop_glb::ShopGlbEmbeddedCamera> {
    let by = &cpu.embedded_cameras_by_name;
    if boss_blind {
        by.get("boss")
            .copied()
            .or_else(|| by.get("default").copied())
            .or(cpu.embedded_perspective_camera)
    } else {
        by.get("default")
            .copied()
            .or(cpu.embedded_perspective_camera)
    }
}

/// Which play-button mesh to use for hit targets / HUD anchors on the pick-blind scene.
#[inline]
pub fn hallway_pick_blind_play_button_node(boss_blind: bool) -> &'static str {
    if !boss_blind {
        return BTN_PLAY_ROUND;
    }
    with_hallway_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return BTN_PLAY_ROUND;
        };
        let has_boss_btn = shop_glb::marker_translation(cpu, BTN_PLAY_BOSS).is_some()
            || cpu.marker_mesh_bounds_doc_for(BTN_PLAY_BOSS).is_some();
        if has_boss_btn {
            BTN_PLAY_BOSS
        } else {
            BTN_PLAY_ROUND
        }
    })
}

/// Applied in [`crate::render::wgpu_renderer::WgpuRenderer`] when writing hallway env uniforms:
/// multiplies `tile_seed` on top of the shared shop/storeroom exposure path.
pub const HALLWAY_ENV_LINEAR_EXPOSURE_MUL: f32 = 2.35;

/// Minimum `decal_atlas_uv.x` (hemispheric fill in `shop_glb.wgsl`) for this room; `max` with debug tune.
pub const HALLWAY_ENV_AMBIENT_SCALE_MIN: f32 = 0.085;

// ── Pick-blind hallway vertex distortion (`shop_glb.wgsl` / `tile_3d.wgsl` @group(0) @binding(8)) ──

/// Base bend amplitude along the lateral axis (world units, scaled by mask / intensity).
pub const HALLWAY_BOW_AMOUNT: f32 = 0.070;
/// Spatial frequency of the bow along corridor depth (1 / world unit).
pub const HALLWAY_BOW_FREQ: f32 = 0.50;
/// Inward/outward wall pulse amplitude (world units).
pub const HALLWAY_BREATHE_AMOUNT: f32 = 0.011;
pub const HALLWAY_BREATHE_FREQ: f32 = 0.78;
/// How strongly ceiling vertices drop toward the floor (per unit above threshold Z).
pub const HALLWAY_CEILING_SQUEEZE: f32 = 0.055;
/// World Z above which ceiling squeeze starts (see `HallwayDistortion::ceiling[1]`).
pub const HALLWAY_CEILING_THRESHOLD_Z: f32 = 0.72;
/// Push vertices along the depth axis (fake length).
pub const HALLWAY_STRETCH_AMOUNT: f32 = 0.07;
/// Max twist angle (radians) at the far mask.
pub const HALLWAY_TWIST_RAD: f32 = 0.095;
/// `smoothstep` depth range in world units along the chosen depth axis (+Y for hallway.glb).
/// Used as a fallback until [`hallway_distortion_apply_glb_depth_extent`] overwrites [`HallwayDistortion::mask`].z/w.
pub const HALLWAY_DEPTH_MASK_START: f32 = 0.12;
pub const HALLWAY_DEPTH_MASK_END: f32 = 14.0;
/// Depth axis selector for `mask[0]`: 0 = +X, 1 = +Y, 2 = +Z (after applying `mask[1]` sign).
pub const HALLWAY_DEPTH_AXIS: f32 = 1.0;
pub const HALLWAY_DEPTH_SIGN: f32 = 1.0;
pub const HALLWAY_DRIFT_SPEED: f32 = 0.018;
pub const HALLWAY_PULSE_SPEED: f32 = 0.9;
pub const HALLWAY_PULSE_AMOUNT: f32 = 0.12;
pub const HALLWAY_CEILING_PULSE_AMT: f32 = 0.22;
pub const HALLWAY_CEILING_PULSE_FREQ: f32 = 1.4;

#[inline]
fn hallway_depth_axis_doc() -> Vec3 {
    let v = if HALLWAY_DEPTH_AXIS < 0.5 {
        Vec3::X
    } else if HALLWAY_DEPTH_AXIS < 1.5 {
        Vec3::Y
    } else {
        Vec3::Z
    };
    v * HALLWAY_DEPTH_SIGN
}

/// Uniform block consumed by `shop_glb.wgsl` / `tile_3d.wgsl` vertex stage (`binding(8)`).
/// When `flags[0] < 0.5`, distortion is a no-op (shop / archive / tiles use a zeroed buffer).
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HallwayDistortion {
    /// x = bow lateral scale, y = spatial frequency, z = bow direction sign (±1), w = bow phase (rad).
    pub bow: [f32; 4],
    /// x = breathe amplitude, y = angular frequency, z = phase (rad), w = side falloff power.
    pub breathe: [f32; 4],
    /// x = squeeze strength, y = ceiling Z threshold, z = ceiling pulse amount, w = ceiling pulse freq.
    pub ceiling: [f32; 4],
    /// x = stretch along depth axis, y = mask start, z = mask end, w = stretch bias scale.
    pub stretch: [f32; 4],
    /// x = max twist magnitude (rad), y = twist mask power, z = depth mask profile (0 = legacy
    /// smoothstep ramp along +depth; 1 = bell curve centered on [`HallwayDistortion::mask`] z/w
    /// span from GLB bounds). w = twist sign (+1 or −1); negates rotation so the hall can twist
    /// either way around the depth axis (see `shop_glb.wgsl` / `tile_3d.wgsl`).
    pub twist: [f32; 4],
    /// x = depth axis index (0..2), y = depth sign (±1), z/w = depth span along that axis in
    /// world units (see [`hallway_distortion_apply_glb_depth_extent`]; shaders interpret with
    /// [`HallwayDistortion::twist`].z as either a ramp or a GLB-centered bell).
    pub mask: [f32; 4],
    /// x = time (sec, filled by renderer), y = drift speed, z = pulse speed, w = pulse amount.
    pub time_pulse: [f32; 4],
    /// x = enabled (1), y = boss pressure 0..1 (`1` on boss blind — red-shifted punctual lights;
    /// vertex warp scales this by 0.25 in WGSL so distortion strength stays tuned), z =
    /// global_intensity, w = seed hash unit 0..1 (unused in shaders; keeps GPU block layout).
    pub flags: [f32; 4],
}

#[inline]
fn loop_seed_hash(run_number: u32, ante: u32, blind: BlindKind) -> u32 {
    let b = match blind {
        BlindKind::Small => 0u32,
        BlindKind::Big => 1u32,
        BlindKind::Boss => 2u32,
    };
    let mut x = run_number.wrapping_mul(0x9E37_79B1u32)
        ^ ante.wrapping_mul(0x85EB_CA6Bu32)
        ^ b.wrapping_mul(0xC2B2_AE35u32);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

#[inline]
fn u01(x: u32) -> f32 {
    (x as f32) * (1.0 / 4294967296.0)
}

impl HallwayDistortion {
    /// Build distortion parameters for the pick-blind hallway. Wall-clock time (seconds) is
    /// merged in [`WgpuRenderer::write_hallway_environment_uniforms`](crate::render::wgpu_renderer::WgpuRenderer).
    pub fn from_pick_blind(blind: BlindKind, run_number: u32, ante: u32) -> Self {
        let h = loop_seed_hash(run_number, ante, blind);
        let bow_dir = if h & 1 == 0 { -1.0 } else { 1.0 };
        let bow_phase = u01(h.rotate_left(3)) * std::f32::consts::TAU;
        let breathe_phase = u01(h.rotate_left(7)) * std::f32::consts::TAU;
        let pulse_phase_off = u01(h.rotate_left(11)) * std::f32::consts::TAU;
        let bow_freq_jitter = 0.88 + u01(h.rotate_left(13)) * 0.28;
        let breathe_freq_jitter = 0.88 + u01(h.rotate_left(17)) * 0.22;
        let stretch_bias = 0.82 + u01(h.rotate_left(19)) * 0.36;
        let twist_bias_pow = 0.72 + u01(h.rotate_left(23)) * 0.38;
        let twist_handed = if h.rotate_left(27) & 1 == 0 {
            1.0f32
        } else {
            -1.0f32
        };

        let (global_intensity, bow_mul, breathe_mul, stretch_mul, twist_mul) = match blind {
            BlindKind::Small => (0.58, 0.85, 0.58, 0.7, 0.52),
            BlindKind::Big => (0.74, 0.95, 0.68, 0.85, 0.72),
            BlindKind::Boss => (0.25, 1.12, 0.82, 1.15, 1.08),
        };
        let boss_pressure = match blind {
            BlindKind::Boss => 1.0,
            BlindKind::Small | BlindKind::Big => 0.0,
        };

        Self {
            bow: [
                HALLWAY_BOW_AMOUNT * bow_mul * global_intensity,
                HALLWAY_BOW_FREQ * bow_freq_jitter,
                bow_dir,
                bow_phase + pulse_phase_off * 0.15,
            ],
            breathe: [
                HALLWAY_BREATHE_AMOUNT * breathe_mul * global_intensity,
                HALLWAY_BREATHE_FREQ * breathe_freq_jitter,
                breathe_phase,
                1.35,
            ],
            ceiling: [
                HALLWAY_CEILING_SQUEEZE * global_intensity,
                HALLWAY_CEILING_THRESHOLD_Z,
                HALLWAY_CEILING_PULSE_AMT,
                HALLWAY_CEILING_PULSE_FREQ,
            ],
            stretch: [
                HALLWAY_STRETCH_AMOUNT * stretch_mul * global_intensity,
                HALLWAY_DEPTH_MASK_START,
                HALLWAY_DEPTH_MASK_END,
                stretch_bias,
            ],
            twist: [
                HALLWAY_TWIST_RAD * twist_mul * global_intensity,
                twist_bias_pow,
                0.0,
                twist_handed,
            ],
            mask: [
                HALLWAY_DEPTH_AXIS,
                HALLWAY_DEPTH_SIGN,
                HALLWAY_DEPTH_MASK_START,
                HALLWAY_DEPTH_MASK_END,
            ],
            time_pulse: [
                0.0,
                HALLWAY_DRIFT_SPEED,
                HALLWAY_PULSE_SPEED,
                HALLWAY_PULSE_AMOUNT * global_intensity,
            ],
            flags: [1.0, boss_pressure, global_intensity, u01(h.rotate_left(29))],
        }
    }

    /// Extra amplitude / timing scales from the debug hallway overlay (`HallwayDistortionDebugSnapshot`).
    pub fn apply_debug_snapshot_scales(&mut self, s: HallwayDistortionDebugSnapshot) {
        let g = s.global_mul.max(0.0);
        self.bow[0] *= s.bow_mul.max(0.0) * g;
        self.breathe[0] *= s.breathe_mul.max(0.0) * g;
        self.ceiling[0] *= s.ceiling_mul.max(0.0) * g;
        self.stretch[0] *= s.stretch_mul.max(0.0) * g;
        self.twist[0] *= s.twist_mul.max(0.0) * g;
        self.time_pulse[3] *= s.pulse_mul.max(0.0);
        self.time_pulse[1] *= s.drift_mul.max(0.0);
        self.ceiling[2] *= s.pulse_mul.max(0.0).min(3.0);
    }
}

/// Carried on [`crate::scenes::DrawCtx`] when **Debug → Hallway hall FX…** is open.
#[derive(Clone, Copy, Debug, Default)]
pub struct HallwayDistortionDebugSnapshot {
    /// `0` = use the run's upcoming blind; `1` Small, `2` Big, `3` Boss.
    pub blind_mode: u8,
    pub seed_run: u32,
    pub seed_ante: u32,
    pub global_mul: f32,
    pub bow_mul: f32,
    pub breathe_mul: f32,
    pub ceiling_mul: f32,
    pub stretch_mul: f32,
    pub twist_mul: f32,
    pub pulse_mul: f32,
    pub drift_mul: f32,
}

impl HallwayDistortionDebugSnapshot {
    /// `upcoming` is used when `blind_mode == 0` (Auto). Run/ante seeds always
    /// come from [`HallwayDistortionDebugSnapshot::seed_run`] /
    /// [`HallwayDistortionDebugSnapshot::seed_ante`] (the overlay sliders).
    pub fn resolve(self, upcoming: BlindKind) -> HallwayDistortion {
        let blind = match self.blind_mode {
            0 => upcoming,
            1 => BlindKind::Small,
            2 => BlindKind::Big,
            _ => BlindKind::Boss,
        };
        let rn = self.seed_run.max(1);
        let an = self.seed_ante.max(1);
        let mut d = HallwayDistortion::from_pick_blind(blind, rn, an);
        d.apply_debug_snapshot_scales(self);
        d
    }
}

enum HallwayGlbCache {
    Uninit,
    Ready(Option<RoomGlbCpu>),
}

static HALLWAY_GLB_CPU: RwLock<HallwayGlbCache> = RwLock::new(HallwayGlbCache::Uninit);

fn ensure_hallway_glb_loaded() {
    let mut w = HALLWAY_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if !matches!(*w, HallwayGlbCache::Uninit) {
        return;
    }
    let ready = if let Some(file) = crate::asset_path::get("3d/hallway.glb") {
        match load_hallway_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::debug!(
                    "hallway.glb: {} marker(s), {} draw primitive(s)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                );
                Some(cpu)
            }
            Err(e) => {
                log::error!("hallway.glb failed to load: {e:#}");
                None
            }
        }
    } else {
        log::warn!("hallway.glb not embedded");
        None
    };
    *w = HallwayGlbCache::Ready(ready);
}

/// Read-only access to decoded hallway data.
pub fn with_hallway_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_hallway_glb_loaded();
    let g = HALLWAY_GLB_CPU.read().unwrap_or_else(|e| e.into_inner());
    match &*g {
        HallwayGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        HallwayGlbCache::Ready(None) => f(None),
        HallwayGlbCache::Uninit => unreachable!(),
    }
}

/// Overwrites [`HallwayDistortion::mask`] `.z`/`.w` with world-space depth extent of the
/// hallway GLB (along [`hallway_depth_axis_doc`]) and sets [`HallwayDistortion::twist`].z to 1
/// so shaders apply a **center-weighted** mask along that span. No-op if bounds are missing.
pub fn hallway_distortion_apply_glb_depth_extent(
    dist: &mut HallwayDistortion,
    window_h: f32,
    env_height_scale: f32,
) {
    with_hallway_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return;
        };
        let Some(bounds) = cpu.environment_bounds_doc else {
            return;
        };
        let m = shop_glb::shop_env_model_matrix_from_cpu(window_h, env_height_scale, cpu);
        let axis_w = m.transform_vector3(hallway_depth_axis_doc());
        let an = axis_w.length();
        if an < 1e-8 {
            return;
        }
        let axis_n = axis_w / an;
        let mut d_min = f32::INFINITY;
        let mut d_max = f32::NEG_INFINITY;
        for c in bounds.corners() {
            let p = m.transform_point3(c);
            let d = p.dot(axis_n);
            d_min = d_min.min(d);
            d_max = d_max.max(d);
        }
        if !(d_max > d_min + 1e-4) {
            return;
        }
        let span = d_max - d_min;
        let pad = span * 0.04;
        dist.mask[2] = d_min - pad;
        dist.mask[3] = d_max + pad;
        dist.twist[2] = 1.0;
    });
}

/// Drops CPU mesh/texture RAM after GPU upload (same contract as shop).
pub fn release_hallway_environment_cpu_sources_after_gpu_upload() {
    let mut g = HALLWAY_GLB_CPU.write().unwrap_or_else(|e| e.into_inner());
    if let HallwayGlbCache::Ready(Some(cpu)) = &mut *g {
        shop_glb::release_room_environment_primitives_cpu(cpu);
    }
}

#[inline]
fn is_hallway_marker_name(name: &str) -> bool {
    matches!(
        name,
        BTN_PLAY_ROUND | BTN_PLAY_BOSS | BTN_SKIP_ROUND
    )
}

#[derive(Copy, Clone)]
struct HallwayRoomWalkHooks;

impl RoomEnvWalkHooks for HallwayRoomWalkHooks {
    fn is_marker(&self, name: &str) -> bool {
        is_hallway_marker_name(name)
    }

    fn mesh_policy(&self, _name: &str) -> RoomMeshPolicy {
        RoomMeshPolicy::EnvironmentDraw
    }

    fn log_asset_label(&self) -> &'static str {
        "hallway.glb"
    }
}

pub fn load_hallway_glb_from_bytes(data: &[u8]) -> anyhow::Result<RoomGlbCpu> {
    let mut cpu = load_room_glb_from_bytes(
        data,
        "gltf::import_slice(hallway.glb)",
        "hallway.glb has no scenes",
        &HallwayRoomWalkHooks,
    )?;
    cpu.collision_meshes.clear();
    Ok(cpu)
}

/// World-space marker position (same centering + scale as uploaded hallway mesh).
pub fn hallway_marker_world(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> Option<Vec3> {
    let t = shop_glb::marker_translation(cpu, name)?;
    let s = shop_glb::shop_env_world_scale(window_h, env_height_scale);
    Some(t * s)
}

/// First embedded perspective camera (shop preferred name or depth-first fallback).
#[cfg_attr(not(test), allow(dead_code))]
pub fn hallway_camera_from_glb_if_present(
    window_h: f32,
    env_height_scale: f32,
) -> Option<CameraParams> {
    with_hallway_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        cpu.embedded_perspective_camera
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

/// Embedded perspective for pick-blind, honoring glTF cameras named `default` / `boss`.
pub fn hallway_pick_blind_embedded_camera_params(
    window_h: f32,
    env_height_scale: f32,
    boss_blind: bool,
) -> Option<CameraParams> {
    with_hallway_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        pick_blind_embedded_camera_doc(cpu, boss_blind)
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

fn hallway_camera_resolve(w: f32, h: f32, env_h: f32, from_glb: Option<CameraParams>) -> CameraParams {
    with_hallway_glb_cpu(|opt| {
        let mut cam = from_glb.unwrap_or_else(|| CameraParams {
            eye: [0.0, -h * 1.25, h * 0.50],
            target: [0.0, h * 0.05, h * 0.18],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 55.0,
            clip_near: None,
            clip_far: None,
        });
        if from_glb.is_none() {
            if let Some(cpu) = opt {
                let corners = shop_glb::shop_world_bounds_corners_centered(h, env_h, cpu);
                cam = shop_glb::shop_camera_fit_fovy_for_corners(w, h, cam, &corners, 0.94);
            }
        }
        if let Some(cpu) = opt {
            cam = shop_glb::shop_camera_with_room_clip_planes(cam, h, env_h, cpu);
        }
        cam
    })
}

/// Camera for pick-blind: `default` / `boss` glTF cameras when present, else same as [`hallway_camera_base`].
pub fn hallway_camera_pick_blind(w: f32, h: f32, env_h: f32, boss_blind: bool) -> CameraParams {
    // Resolve outside nested `with_hallway_glb_cpu` — avoids deadlock on first load.
    let from_glb = hallway_pick_blind_embedded_camera_params(h, env_h, boss_blind);
    hallway_camera_resolve(w, h, env_h, from_glb)
}

/// Legacy pick-blind camera (single embedded cam or bounds fit). Unit tests use this; gameplay uses [`hallway_camera_pick_blind`].
#[cfg_attr(not(test), allow(dead_code))]
pub fn hallway_camera_base(w: f32, h: f32, env_h: f32) -> CameraParams {
    let from_glb = hallway_camera_from_glb_if_present(h, env_h);
    hallway_camera_resolve(w, h, env_h, from_glb)
}

pub fn hallway_glb_has_embedded_lights() -> bool {
    with_hallway_glb_cpu(|opt| {
        opt.is_some_and(|cpu| {
            !cpu.embedded_point_lights.is_empty() || !cpu.embedded_spot_lights.is_empty()
        })
    })
}

fn gltf_punctual_linear_rgb(
    raw: [f32; 3],
    is_candle: bool,
    tune: &ShopEnvLightingTune,
) -> [f32; 3] {
    if is_candle {
        [
            (raw[0] * tune.candle_light_color_mul[0]).clamp(0.0, 1.0),
            (raw[1] * tune.candle_light_color_mul[1]).clamp(0.0, 1.0),
            (raw[2] * tune.candle_light_color_mul[2]).clamp(0.0, 1.0),
        ]
    } else {
        raw
    }
}

/// glTF punctual points merged into [`crate::render::draw_cmd::SceneLighting::punctual`] (hallway room).
pub fn hallway_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &ShopEnvLightingTune,
) -> Vec<PointLight> {
    with_hallway_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return Vec::new();
        };
        if cpu.embedded_point_lights.is_empty() {
            return Vec::new();
        }
        let s = shop_glb::shop_env_world_scale(h, env_h);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        let budget = MAX_POINT_LIGHTS.saturating_sub(2);
        if cpu.embedded_point_lights.len() > budget {
            log::warn!(
                "hallway.glb: {} point lights exceed budget ({}) — truncating",
                cpu.embedded_point_lights.len(),
                budget
            );
        }
        cpu.embedded_point_lights
            .iter()
            .take(budget)
            .map(|l| {
                let world = (l.pos_doc - center_doc) * s;
                let radius = glb_punctual_range_world_upload(h, s, l.range_doc);
                PointLight {
                    pos: surface_anchor_from_world_xyz(w, h, world),
                    radius,
                    color: gltf_punctual_linear_rgb(l.color_linear, l.is_candle, tune),
                    intensity: (l.intensity * tune.gltf_light_intensity_scale).max(0.0),
                }
            })
            .collect()
    })
}

/// glTF spot lights for [`UiFrame::spot_lights`].
pub fn hallway_embedded_spot_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &ShopEnvLightingTune,
) -> Vec<SpotLight> {
    if !hallway_glb_has_embedded_lights() {
        return Vec::new();
    }
    with_hallway_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return Vec::new();
        };
        if cpu.embedded_spot_lights.is_empty() {
            return Vec::new();
        }
        let s = shop_glb::shop_env_world_scale(h, env_h);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        if cpu.embedded_spot_lights.len() > MAX_SPOT_LIGHTS {
            log::warn!(
                "hallway.glb: {} spot lights exceed {} — truncating",
                cpu.embedded_spot_lights.len(),
                MAX_SPOT_LIGHTS
            );
        }
        cpu.embedded_spot_lights
            .iter()
            .take(MAX_SPOT_LIGHTS)
            .filter_map(|l| {
                let dir_w = l.dir_doc.normalize_or_zero();
                if dir_w.length_squared() < 1e-12 {
                    return None;
                }
                let world = (l.pos_doc - center_doc) * s;
                let radius = glb_punctual_range_world_upload(h, s, l.range_doc);
                let cos_outer = l.outer_cone_rad.cos();
                let cos_inner = l.inner_cone_rad.cos().max(cos_outer);
                Some(SpotLight {
                    pos: surface_anchor_from_world_xyz(w, h, world),
                    dir: dir_w.to_array(),
                    radius,
                    cos_outer,
                    cos_inner,
                    color: gltf_punctual_linear_rgb(l.color_linear, l.is_candle, tune),
                    intensity: (l.intensity * tune.gltf_light_intensity_scale).max(0.0),
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::load_hallway_glb_from_bytes;
    use crate::render::shop_glb;

    /// Pick-blind room (`hallway.glb`) — documents how many environment primitives carry glTF
    /// emissive; re-run after authoring so the count reflects `emissiveTexture` / factor.
    #[test]
    fn pick_blind_camera_uses_tight_clip_planes() {
        let w = 1920.0;
        let h = 1080.0;
        let cam = super::hallway_camera_base(w, h, 1.0);
        let (near, far) = cam.clip_planes(h);
        let ratio = far / near;
        let legacy_ratio = (h * crate::render::draw_cmd::SCENE_PERSPECTIVE_FAR_MUL) / 1.0;
        assert!(
            near >= crate::render::room_env_gltf::ROOM_CAMERA_CLIP_NEAR_MIN,
            "inside-room camera should keep a low near plane, got {near}"
        );
        assert!(
            near <= 500.0,
            "near must not use far AABB corners when the camera is inside the room, got {near}"
        );
        assert!(
            ratio < legacy_ratio * 0.05,
            "far/near ratio should be much tighter than legacy {legacy_ratio:.0}, got {ratio:.1}"
        );
    }

    #[test]
    fn pick_blind_room_emissive_material_summary() {
        let data = match crate::asset_path::get("3d/hallway.glb") {
            Some(f) => f.data,
            None => {
                eprintln!(
                    "skip pick_blind_room_emissive_material_summary: no 3d/hallway.glb (bake packs or set MAHJURO_ASSETS)"
                );
                return;
            }
        };
        let cpu = load_hallway_glb_from_bytes(&data).expect("hallway.glb decode");
        let mut with_tex = 0usize;
        let mut with_factor = 0usize;
        for ep in &cpu.environment_primitives {
            let m = &ep.mesh;
            if m.emissive_rgba.is_some() {
                with_tex += 1;
            }
            let f = m.emissive_factor;
            if f[0] > 1e-5 || f[1] > 1e-5 || f[2] > 1e-5 {
                with_factor += 1;
            }
        }
        eprintln!(
            "hallway.glb (pick-blind): {} env primitive(s), {} with emissive texture, {} with non-zero emissive factor",
            cpu.environment_primitives.len(),
            with_tex,
            with_factor
        );
    }

    /// When `3d/hallway.glb` is available (baked packs / `MAHJURO_ASSETS`), assert pick-blind boss
    /// authoring: named `default` / `boss` cameras, distinct framing, and `btn_play_boss` marker data.
    #[test]
    fn pick_blind_hallway_boss_camera_and_play_boss_marker() {
        let Some(file) = crate::asset_path::get("3d/hallway.glb") else {
            eprintln!(
                "skip pick_blind_hallway_boss_camera_and_play_boss_marker: no 3d/hallway.glb (bake packs or set MAHJURO_ASSETS)"
            );
            return;
        };
        let cpu = load_hallway_glb_from_bytes(&file.data).expect("hallway.glb decode");
        let by = &cpu.embedded_cameras_by_name;
        if !by.contains_key("default") || !by.contains_key("boss") {
            let keys: Vec<&String> = by.keys().collect();
            eprintln!(
                "skip pick_blind_hallway_boss_camera_and_play_boss_marker: need perspective camera nodes named `default` and `boss` (got {} named camera(s): {:?})",
                keys.len(),
                keys
            );
            return;
        }
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        let h = 1080.0_f32;
        let env_h = 1.0_f32;
        let cam_default = by["default"].to_camera_params(h, env_h, center_doc);
        let cam_boss = by["boss"].to_camera_params(h, env_h, center_doc);
        let diff_eye: f32 = cam_default
            .eye
            .iter()
            .zip(cam_boss.eye.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        let diff_tgt: f32 = cam_default
            .target
            .iter()
            .zip(cam_boss.target.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(
            diff_eye + diff_tgt > 1e-3,
            "hallway `default` and `boss` cameras should differ in eye/target for pick_blind; got eye_sum_abs_diff={diff_eye} target_sum_abs_diff={diff_tgt}"
        );

        let has_boss_play = shop_glb::marker_translation(&cpu, super::BTN_PLAY_BOSS).is_some()
            || cpu
                .marker_mesh_bounds_doc_for(super::BTN_PLAY_BOSS)
                .is_some();
        if !has_boss_play {
            eprintln!(
                "skip pick_blind_hallway_boss_camera_and_play_boss_marker: add `btn_play_boss` (transform and/or mesh) for boss pick-blind play hit target"
            );
            return;
        }

        let cam_pb_false = super::hallway_camera_pick_blind(1920.0, 1080.0, env_h, false);
        let cam_pb_true = super::hallway_camera_pick_blind(1920.0, 1080.0, env_h, true);
        let d: f32 = cam_pb_false
            .eye
            .iter()
            .zip(cam_pb_true.eye.iter())
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            + cam_pb_false
                .target
                .iter()
                .zip(cam_pb_true.target.iter())
                .map(|(a, b)| (a - b).abs())
                .sum::<f32>();
        assert!(
            d > 1e-3,
            "hallway_camera_pick_blind(boss=false) vs boss=true should differ when default/boss cameras differ; got combined_abs_diff={d}"
        );
    }
}
