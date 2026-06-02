//! [`hallway.glb`](../../../assets/3d/hallway.glb) — pick-blind hallway room.
//!
//! Marker object names (Blender → glTF):
//! - `btn_play_round` — commit to the upcoming blind (Small/Big).
//! - `btn_play_boss` — optional boss-only play control (falls back to `btn_play_round` if absent).
//! - `btn_skip_round` — skip for the tribute tag when allowed (non-boss).
//! - Perspective cameras named `default` and `boss` select pick-blind framing (see
//!   [`hallway_camera_pick_chamber`]); legacy single-camera files still use the first embedded camera.
//!
//! Decodes through [`crate::room_env_gltf`]; decoded layout matches [`crate::room_glb::RoomGlbCpu`]
//! for the shared GPU path (`room_glb.wgsl` / embedded lights).

use parking_lot::RwLock;

use glam::Vec3;

use mahjuro_core::core::rules::ChamberKind;
use crate::draw_cmd::CameraParams;
use crate::room_env_gltf::{RoomEnvWalkHooks, RoomMeshPolicy};
use crate::room_glb::{self, RoomEnvLightingTune, RoomGlbCpu, load_room_glb_from_bytes};
use crate::wgpu_renderer::{PointLight, SpotLight};

/// glTF node names for pick-blind actions (must match Blender objects).
pub const BTN_PLAY_ROUND: &str = "btn_play_round";
pub const BTN_PLAY_BOSS: &str = "btn_play_boss";
pub const BTN_SKIP_ROUND: &str = "btn_skip_round";

/// glTF mesh node for pick-blind hallway wall panels (tinted per run via [`HallwayDistortion::bow`]).
pub const HALLWAY_WALLS_NODE: &str = "walls";

/// `COLOR_0.a` on [`HALLWAY_WALLS_NODE`] — `room_glb.wgsl` multiplies albedo by `bow.rgb` (alpha treated as 1).
/// Must differ from [`crate::room_env_gltf::ROOM_ENV_COLOR_A_ARCHIVE_NO_DIRECTIONAL_SHADOW`]
/// so hallway walls still receive punctual shadows.
pub const HALLWAY_WALL_TINT_COLOR_TAG: f32 =
    crate::room_env_gltf::ROOM_ENV_COLOR_A_HALLWAY_WALL_TINT;

/// Linear RGB tints (blue, yellow, green, red, purple, orange, pink, brown).
const HALLWAY_WALL_TINTS: [[f32; 3]; 8] = [
    [0.35, 0.55, 0.95],
    [0.95, 0.82, 0.28],
    [0.38, 0.78, 0.42],
    [0.92, 0.35, 0.32],
    [0.68, 0.42, 0.88],
    [0.95, 0.52, 0.22],
    [0.95, 0.55, 0.75],
    [0.72, 0.48, 0.32],
];

#[inline]
fn pick_chamber_embedded_camera_doc(
    cpu: &RoomGlbCpu,
    ordeal_chamber: bool,
) -> Option<room_glb::RoomGlbEmbeddedCamera> {
    let by = &cpu.embedded_cameras_by_name;
    if ordeal_chamber {
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
pub fn hallway_pick_chamber_play_button_node(ordeal_chamber: bool) -> &'static str {
    if !ordeal_chamber {
        return BTN_PLAY_ROUND;
    }
    with_hallway_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return BTN_PLAY_ROUND;
        };
        let has_boss_btn = room_glb::marker_translation(cpu, BTN_PLAY_BOSS).is_some()
            || cpu.marker_mesh_bounds_doc_for(BTN_PLAY_BOSS).is_some();
        if has_boss_btn {
            BTN_PLAY_BOSS
        } else {
            BTN_PLAY_ROUND
        }
    })
}

/// Applied in [`crate::wgpu_renderer::WgpuRenderer`] when writing hallway env uniforms:
/// multiplies `tile_seed` on top of the shared shop/storeroom exposure path.
pub const HALLWAY_ENV_LINEAR_EXPOSURE_MUL: f32 = 1.0;

/// Hemispheric fill in `room_glb.wgsl` (`decal_atlas_uv.x`). Windowless interior — no sky ambient.
pub const HALLWAY_ENV_AMBIENT_SCALE_MIN: f32 = 0.0;

// ── Pick-blind hallway vertex distortion (`room_glb.wgsl` / `tile_3d.wgsl` @group(0) @binding(8)) ──

/// Inward/outward wall pulse amplitude (world units).
pub const HALLWAY_BREATHE_AMOUNT: f32 = 0.011;
pub const HALLWAY_BREATHE_FREQ: f32 = 0.78;
/// How strongly ceiling vertices drop toward the floor (per unit above threshold Z).
pub const HALLWAY_CEILING_SQUEEZE: f32 = 0.11;
/// World Z above which ceiling squeeze starts (see `HallwayDistortion::ceiling[1]`).
pub const HALLWAY_CEILING_THRESHOLD_Z: f32 = 0.72;
/// Push vertices along the depth axis (fake length).
pub const HALLWAY_STRETCH_AMOUNT: f32 = 0.07;
/// Max twist angle (radians) at the far mask.
pub const HALLWAY_TWIST_RAD: f32 = 0.19;
/// `smoothstep` depth range in world units along the chosen depth axis (+Y for hallway.glb).
/// Used as a fallback until [`hallway_distortion_apply_glb_depth_extent`] overwrites [`HallwayDistortion::mask`].z/w.
pub const HALLWAY_DEPTH_MASK_START: f32 = 0.12;
pub const HALLWAY_DEPTH_MASK_END: f32 = 14.0;
/// Depth axis selector for `mask[0]`: 0 = +X, 1 = +Y, 2 = +Z (after applying `mask[1]` sign).
pub const HALLWAY_DEPTH_AXIS: f32 = 1.0;
pub const HALLWAY_DEPTH_SIGN: f32 = 1.0;
pub const HALLWAY_DRIFT_SPEED: f32 = 0.036;
pub const HALLWAY_PULSE_SPEED: f32 = 0.9;
pub const HALLWAY_PULSE_AMOUNT: f32 = 0.24;
pub const HALLWAY_CEILING_PULSE_AMT: f32 = 0.44;
pub const HALLWAY_CEILING_PULSE_FREQ: f32 = 1.4;
/// Fallback lateral half-width (world units) for twist until GLB bounds overwrite [`HallwayDistortion::flags`].w.
pub const HALLWAY_LATERAL_HALF_WIDTH: f32 = 2.0;
/// Lateral wall ripple amplitude (world units along corridor `lateral` axis).
pub const HALLWAY_RIPPLE_AMOUNT: f32 = 0.019;
/// Wave count along normalized corridor depth `u` (see `ripple.y` in WGSL).
pub const HALLWAY_RIPPLE_WAVES: f32 = 5.0;
/// Traveling ripple speed (rad/s; scaled by `HALLWAY_ANIM_TIME_SCALE` in shader).
pub const HALLWAY_RIPPLE_SPEED: f32 = 1.2;
/// `mix(standing, traveling, w)` — higher = more travel down the hall.
pub const HALLWAY_RIPPLE_TRAVEL_MIX: f32 = 0.7;
/// Last wing index for hallway intensity ramp (keep in sync with `run::FINAL_WING`).
pub const HALLWAY_WING_FINAL: u32 = 7;
/// Pick-blind warp scale at wing 1 vs [`HALLWAY_WING_FINAL`].
pub const HALLWAY_WING_INTENSITY_AT_FIRST: f32 = 0.82;
pub const HALLWAY_WING_INTENSITY_AT_FINAL: f32 = 1.28;
/// Wall barrel-bow displacement at the wall surface (world units; shader `bow.w`).
pub const HALLWAY_BALLOON_AMOUNT: f32 = 0.065;
/// World Z for vertical barrel midline in WGSL (`mix(floor, ceiling, 0.5)`); keep in sync with shaders.
pub const HALLWAY_BALLOON_FLOOR_Z: f32 = 0.08;

// Left/right in `room_glb.wgsl` / `tile_3d.wgsl` (all use the same lateral frame):
// - `lateral = normalize(cross(depth_axis, world_up))` (handedness follows depth sign in `mask.y`).
// - `side_c = dot(world, lateral) - stretch.y` with `stretch.y` = glTF root lateral coord (not AABB center).
// - `side_n = clamp(side_c / flags.w, −1, 1)`; `flags.w` = lateral corridor half-width (bounds span).
// - Twist: rigid spiral around depth (`u`); sign from `twist.w` (+1 / −1 per run). Not `side_n`.
// - `twist.w` flips rotation hand; breathe/pulse depth phase uses normalized `u` along `mask` span.
// - `stretch.z` stores glTF root depth (CPU). `bow.rgb` = per-run wall tint; `bow.w` = balloon amp.
// - `ripple` = lateral waves.
// - Stretch displaces along depth × `u` (not a rigid translate).
// - `ripple`: x = amplitude, y = waves along `u`, z = travel speed, w = travel mix (0 standing .. 1 travel).

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

/// Uniform block consumed by `room_glb.wgsl` / `tile_3d.wgsl` vertex stage (`binding(8)`)
/// and by `shadow.wgsl` depth pass (`@group(1) @binding(0)`).
/// When `flags[0] < 0.5`, distortion is a no-op (shop / archive / tiles use a zeroed buffer).
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HallwayDistortion {
    /// Pick-blind wall tint (linear RGB). `w` = wall barrel-bow strength (× lateral half-width).
    /// Only meshes tagged with [`HALLWAY_WALL_TINT_COLOR_TAG`] multiply albedo by `bow.rgb`.
    pub bow: [f32; 4],
    /// x = breathe amplitude, y = angular frequency, z = phase (rad), w = side falloff power.
    pub breathe: [f32; 4],
    /// x = squeeze strength, y = ceiling Z threshold, z = ceiling pulse amount, w = ceiling pulse freq.
    pub ceiling: [f32; 4],
    /// x = stretch along depth axis, y = glTF-root lateral coord (`dot(origin, lateral)` in world),
    /// z = glTF-root depth coord (`dot(origin, depth_axis)` for `y_rel`), w = stretch bias scale.
    pub stretch: [f32; 4],
    /// x = max twist magnitude (rad) at far end of corridor, y = twist mask power, z = depth mask profile (0 = legacy
    /// smoothstep ramp along +depth; 1 = bell curve centered on [`HallwayDistortion::mask`] z/w
    /// span from GLB bounds). w = twist handedness (+1 or −1); whole hall spirals CW or CCW around depth.
    pub twist: [f32; 4],
    /// x = depth axis index (0..2), y = depth sign (±1), z/w = depth span along that axis in
    /// world units (see [`hallway_distortion_apply_glb_depth_extent`]; shaders interpret with
    /// [`HallwayDistortion::twist`].z as either a ramp or a GLB-centered bell).
    pub mask: [f32; 4],
    /// x = time (sec, filled by renderer), y = drift speed, z = pulse speed, w = pulse amount.
    pub time_pulse: [f32; 4],
    /// x = enabled (1), y = boss pressure 0..1 (`1` on boss blind — red-shifted punctual lights;
    /// vertex warp scales this by 0.25 in WGSL so distortion strength stays tuned), z =
    /// global_intensity, w = corridor lateral half-width in world units (`(lat_max − lat_min) / 2`).
    pub flags: [f32; 4],
    /// x = lateral ripple amplitude, y = wave count along depth `u`, z = travel speed (rad/s),
    /// w = travel mix (`0` = standing corrugation, `1` = waves march down the hall).
    pub ripple: [f32; 4],
}

#[inline]
fn u32_from_seed64(mut x: u64) -> u32 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afed_558c_cd65);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^= x >> 33;
    x as u32
}

#[inline]
fn loop_seed_hash(run_seed: u64, run_number: u32, wing: u32, blind: ChamberKind) -> u32 {
    let b = match blind {
        ChamberKind::Small => 0u32,
        ChamberKind::Big => 1u32,
        ChamberKind::Ordeal => 2u32,
    };
    let mut x = u32_from_seed64(
        run_seed
            ^ (run_number as u64).wrapping_mul(0x9E37_79B1_85EB_CA87)
            ^ (wing as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ (b as u64).wrapping_mul(0x1656_67B1_9E37_79F9),
    );
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

#[inline]
fn u01(x: u32) -> f32 {
    (x as f32) * (1.0 / 4294967296.0)
}

#[inline]
fn wall_tint_seed_hash(run_seed: u64, wing: u32) -> u32 {
    let mut x = u32_from_seed64(run_seed ^ (wing as u64).wrapping_mul(0x7F4A_7C15_1656_67B1));
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

/// Number of preset wall tints in [`hallway_wall_tint_rgb`].
pub const HALLWAY_WALL_TINT_COUNT: usize = HALLWAY_WALL_TINTS.len();

/// One of eight hallway wall tints, stable for a given run seed and wing.
#[inline]
pub fn hallway_wall_tint_rgb(run_seed: u64, wing: u32) -> [f32; 3] {
    hallway_wall_tint_by_index(
        (wall_tint_seed_hash(run_seed, wing) as usize) % HALLWAY_WALL_TINT_COUNT,
    )
}

/// Preset wall tint by index `0..[`HALLWAY_WALL_TINT_COUNT`]`.
#[inline]
pub fn hallway_wall_tint_by_index(idx: usize) -> [f32; 3] {
    HALLWAY_WALL_TINTS[idx % HALLWAY_WALL_TINT_COUNT]
}

/// Linear ramp on current wing (1-indexed). Wings above [`HALLWAY_WING_FINAL`] clamp to the final value.
#[inline]
pub fn hallway_wing_intensity_scale(wing: u32) -> f32 {
    if HALLWAY_WING_FINAL <= 1 {
        return HALLWAY_WING_INTENSITY_AT_FINAL;
    }
    let wing = wing.max(1);
    let t = ((wing - 1) as f32 / (HALLWAY_WING_FINAL - 1) as f32).clamp(0.0, 1.0);
    HALLWAY_WING_INTENSITY_AT_FIRST
        + t * (HALLWAY_WING_INTENSITY_AT_FINAL - HALLWAY_WING_INTENSITY_AT_FIRST)
}

fn tag_hallway_walls_for_runtime_tint(cpu: &mut RoomGlbCpu) {
    for ep in &mut cpu.environment_primitives {
        if ep.gltf_node_name.as_deref() == Some(HALLWAY_WALLS_NODE) {
            for v in &mut ep.mesh.vertices {
                v.color[3] = HALLWAY_WALL_TINT_COLOR_TAG;
            }
        }
    }
}

impl HallwayDistortion {
    /// Build distortion parameters for the pick-blind hallway. Wall-clock time (seconds) is
    /// merged in [`WgpuRenderer::write_hallway_environment_uniforms`](crate::wgpu_renderer::WgpuRenderer).
    pub fn from_pick_chamber(
        blind: ChamberKind,
        run_seed: u64,
        run_number: u32,
        wing: u32,
    ) -> Self {
        let h = loop_seed_hash(run_seed, run_number, wing, blind);
        let breathe_phase = u01(h.rotate_left(7)) * std::f32::consts::TAU;
        let breathe_freq_jitter = 0.88 + u01(h.rotate_left(17)) * 0.22;
        let stretch_bias = 0.82 + u01(h.rotate_left(19)) * 0.36;
        let twist_bias_pow = 0.72 + u01(h.rotate_left(23)) * 0.38;
        let twist_handed = if h.rotate_left(27) & 1 == 0 {
            1.0f32
        } else {
            -1.0f32
        };
        let ripple_waves = HALLWAY_RIPPLE_WAVES * (0.9 + u01(h.rotate_left(11)) * 0.2);
        let ripple_travel_mix =
            (HALLWAY_RIPPLE_TRAVEL_MIX + (u01(h.rotate_left(13)) - 0.5) * 0.12).clamp(0.55, 0.85);

        let (mut global_intensity, breathe_mul, stretch_mul, twist_mul, ripple_mul, balloon_mul) =
            match blind {
                ChamberKind::Small => (0.58, 0.58, 0.7, 0.52, 0.5, 0.5),
                ChamberKind::Big => (0.74, 0.68, 0.85, 0.72, 0.75, 0.75),
                ChamberKind::Ordeal => (0.25, 0.82, 1.15, 1.08, 0.4, 1.0),
            };
        global_intensity *= hallway_wing_intensity_scale(wing);
        let boss_pressure = match blind {
            ChamberKind::Ordeal => 1.0,
            ChamberKind::Small | ChamberKind::Big => 0.0,
        };
        let wall_tint = hallway_wall_tint_rgb(run_seed, wing);
        let ripple_amp = (HALLWAY_RIPPLE_AMOUNT * ripple_mul * global_intensity)
            .max(HALLWAY_RIPPLE_AMOUNT * 0.4);
        let balloon_amp = HALLWAY_BALLOON_AMOUNT * balloon_mul * global_intensity;

        Self {
            bow: [wall_tint[0], wall_tint[1], wall_tint[2], balloon_amp],
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
                0.0,
                0.0,
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
            flags: [
                1.0,
                boss_pressure,
                global_intensity,
                HALLWAY_LATERAL_HALF_WIDTH,
            ],
            ripple: [
                ripple_amp,
                ripple_waves,
                HALLWAY_RIPPLE_SPEED,
                ripple_travel_mix,
            ],
        }
    }

    /// Extra amplitude / timing scales from the debug hallway overlay (`HallwayDistortionDebugSnapshot`).
    pub fn apply_debug_snapshot_scales(&mut self, s: HallwayDistortionDebugSnapshot) {
        let g = s.global_mul.max(0.0);
        if s.wall_tint > 0 {
            let idx = (s.wall_tint as usize).saturating_sub(1) % HALLWAY_WALL_TINT_COUNT;
            let tint = hallway_wall_tint_by_index(idx);
            self.bow[0] = tint[0];
            self.bow[1] = tint[1];
            self.bow[2] = tint[2];
        }
        self.breathe[0] *= s.breathe_mul.max(0.0) * g;
        self.ceiling[0] *= s.ceiling_mul.max(0.0) * g;
        self.stretch[0] *= s.stretch_mul.max(0.0) * g;
        self.twist[0] *= s.twist_mul.max(0.0) * g;
        self.flags[2] *= g;
        self.ripple[0] *= s.ripple_mul.max(0.0) * g;
        self.ripple[1] *= s.ripple_waves_mul.max(0.1);
        self.ripple[2] *= s.ripple_mul.max(0.0).sqrt().max(0.25);
        self.ripple[3] = (self.ripple[3] * s.ripple_travel_mul.max(0.0)).clamp(0.0, 1.0);
        self.bow[3] *= s.balloon_mul.max(0.0) * g;
        self.time_pulse[3] *= s.pulse_mul.max(0.0);
        self.time_pulse[1] *= s.drift_mul.max(0.0);
        self.ceiling[2] *= s.pulse_mul.clamp(0.0, 3.0);
    }
}

/// Carried on [`crate::scenes::DrawCtx`] when **Debug → Hallway hall FX…** is open.
#[derive(Clone, Copy, Debug, Default)]
pub struct HallwayDistortionDebugSnapshot {
    /// `0` = use the run's upcoming blind; `1` Small, `2` Big, `3` Boss.
    pub chamber_mode: u8,
    /// Chronicle RNG seed.
    pub run_seed: u64,
    /// Blind counter within the run (see [`HallwayReadModel::run_number`]).
    pub run_number: u32,
    /// Current wing / ante.
    pub wing: u32,
    pub global_mul: f32,
    pub breathe_mul: f32,
    pub ceiling_mul: f32,
    pub stretch_mul: f32,
    pub twist_mul: f32,
    pub pulse_mul: f32,
    pub drift_mul: f32,
    pub ripple_mul: f32,
    pub balloon_mul: f32,
    /// `0` = seed tint; `1..=`[`HALLWAY_WALL_TINT_COUNT`] = forced palette index + 1.
    pub wall_tint: u8,
    pub ripple_waves_mul: f32,
    /// Scales `ripple.w` travel mix (`0` = standing, `1` = marching).
    pub ripple_travel_mul: f32,
}

impl HallwayDistortionDebugSnapshot {
    /// `upcoming` is used when `chamber_mode == 0` (Auto). [`run_seed`] comes from the
    /// overlay (defaults to the active run when the panel is opened).
    pub fn resolve(self, upcoming: ChamberKind) -> HallwayDistortion {
        let blind = match self.chamber_mode {
            0 => upcoming,
            1 => ChamberKind::Small,
            2 => ChamberKind::Big,
            _ => ChamberKind::Ordeal,
        };
        let mut d = HallwayDistortion::from_pick_chamber(
            blind,
            self.run_seed,
            self.run_number.max(1),
            self.wing.max(1),
        );
        d.apply_debug_snapshot_scales(self);
        d
    }
}

enum HallwayGlbCache {
    Uninit,
    Ready(Option<Box<RoomGlbCpu>>),
}

static HALLWAY_GLB_CPU: RwLock<HallwayGlbCache> = RwLock::new(HallwayGlbCache::Uninit);

/// True when `hallway.glb` has been decoded into the process cache.
pub fn hallway_cpu_decoded() -> bool {
    let g = HALLWAY_GLB_CPU.read();
    matches!(&*g, HallwayGlbCache::Ready(Some(_)))
}

/// True when decoded environment meshes are present and not yet released for GPU upload.
pub fn hallway_cpu_ready_for_gpu_upload() -> bool {
    let g = HALLWAY_GLB_CPU.read();
    match &*g {
        HallwayGlbCache::Ready(Some(cpu)) => {
            !cpu.environment_primitives.is_empty() && !cpu.environment_primitives_released
        }
        _ => false,
    }
}

/// Decode `hallway.glb` into the process-wide CPU cache (main or prefetch thread).
pub fn decode_hallway_glb_into_cache() {
    let mut w = HALLWAY_GLB_CPU.write();
    if matches!(&*w, HallwayGlbCache::Ready(Some(cpu)) if !room_glb::room_glb_cpu_needs_environment_mesh_reload(cpu)
        && !room_glb::room_glb_cpu_stale_environment_for_gpu_upload(cpu))
    {
        return;
    }
    let ready = if let Some(file) = mahjuro_assets::asset_path::get("3d/hallway.glb") {
        match load_hallway_glb_from_bytes(&file.data) {
            Ok(cpu) => {
                log::debug!(
                    "hallway.glb: {} marker(s), {} draw primitive(s)",
                    cpu.markers.len(),
                    cpu.environment_primitives.len(),
                );
                Some(cpu)
            }
            Err(e) => panic!("hallway.glb failed to load: {e:#}"),
        }
    } else {
        panic!("hallway.glb not embedded; required when loading hallway room");
    };
    *w = HallwayGlbCache::Ready(ready.map(Box::new));
}

fn ensure_hallway_glb_loaded() {
    crate::room_preload::join_hallway_cpu_prefetch_blocking();
    let mut w = HALLWAY_GLB_CPU.write();
    match &*w {
        HallwayGlbCache::Uninit => {}
        HallwayGlbCache::Ready(Some(cpu))
            if room_glb::room_glb_cpu_needs_environment_mesh_reload(cpu) =>
        {
            *w = HallwayGlbCache::Uninit;
        }
        _ => return,
    }
    drop(w);
    decode_hallway_glb_into_cache();
}

/// Read-only access to decoded hallway data.
pub fn with_hallway_glb_cpu<R>(f: impl FnOnce(Option<&RoomGlbCpu>) -> R) -> R {
    ensure_hallway_glb_loaded();
    let g = HALLWAY_GLB_CPU.read();
    match &*g {
        HallwayGlbCache::Ready(Some(cpu)) => f(Some(cpu)),
        HallwayGlbCache::Ready(None) => f(None),
        HallwayGlbCache::Uninit => unreachable!(),
    }
}

/// Lateral normalization for `side_n` from the `walls` mesh: max |offset| from `lat_ref`
/// (glTF-root lateral plane). Using mesh width or env AABB span inflates `flags[3]` when the
/// walls geometry includes center trim or oversized quads, which zeros ripple/twist on panels.
fn hallway_walls_lateral_half_width(
    cpu: &RoomGlbCpu,
    model: glam::Mat4,
    lateral_n: Vec3,
    lat_ref: f32,
) -> Option<f32> {
    let walls = cpu
        .environment_primitives
        .iter()
        .find(|ep| ep.gltf_node_name.as_deref() == Some(HALLWAY_WALLS_NODE))?;
    let mut dists: Vec<f32> = walls
        .mesh
        .vertices
        .iter()
        .map(|v| {
            let side_c = model
                .transform_point3(Vec3::from(v.position))
                .dot(lateral_n)
                - lat_ref;
            side_c.abs()
        })
        .collect();
    if dists.is_empty() {
        return None;
    }
    dists.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50 = dists[dists.len() / 2];
    let p90_idx = ((dists.len() as f32 * 0.9).floor() as usize).min(dists.len() - 1);
    let p90 = dists[p90_idx];
    // Bimodal walls mesh (panel verts near the corridor + stray far verts): prefer the inner cluster.
    let half = if p90 > p50 * 4.0 { p50 } else { p90 };
    if half > 1e-4 {
        Some(half.clamp(0.25, 24.0))
    } else {
        None
    }
}

/// Fills depth mask span from env bounds, sets lateral/depth reference to the **glTF root**
/// (`model_matrix * vec3(0)` — not the AABB center used to center the drawn mesh), and sets
/// lateral half-width from the [`HALLWAY_WALLS_NODE`] mesh when present (else env bounds span).
pub fn hallway_distortion_apply_glb_depth_extent(
    dist: &mut HallwayDistortion,
    window_h: f32,
    env_height_scale: f32,
) {
    with_hallway_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return;
        };
        let m = room_glb::room_env_model_matrix_from_cpu(window_h, env_height_scale, cpu);
        let origin_world = m.transform_point3(Vec3::ZERO);

        let axis_w = m.transform_vector3(hallway_depth_axis_doc());
        let an = axis_w.length();
        if an < 1e-8 {
            return;
        }
        let axis_n = axis_w / an;
        let mut lateral_n = axis_n.cross(Vec3::Z);
        if lateral_n.length_squared() < 1e-8 {
            lateral_n = Vec3::X;
        } else {
            lateral_n = lateral_n.normalize();
        }

        // Symmetry plane for breathe / twist: glTF local origin, not bounds midpoint.
        dist.stretch[1] = origin_world.dot(lateral_n);
        dist.stretch[2] = origin_world.dot(axis_n);

        let Some(bounds) = cpu.environment_bounds_doc else {
            return;
        };

        let mut d_min = f32::INFINITY;
        let mut d_max = f32::NEG_INFINITY;
        let mut lat_min = f32::INFINITY;
        let mut lat_max = f32::NEG_INFINITY;
        for c in bounds.corners() {
            let p = m.transform_point3(c);
            let d = p.dot(axis_n);
            d_min = d_min.min(d);
            d_max = d_max.max(d);
            let lat = p.dot(lateral_n);
            lat_min = lat_min.min(lat);
            lat_max = lat_max.max(lat);
        }
        if d_max > d_min + 1e-4 {
            let span = d_max - d_min;
            let pad = span * 0.04;
            dist.mask[2] = d_min - pad;
            dist.mask[3] = d_max + pad;
            dist.twist[2] = 1.0;
        }
        if let Some(wall_half) =
            hallway_walls_lateral_half_width(cpu, m, lateral_n, dist.stretch[1])
        {
            dist.flags[3] = wall_half;
        } else if lat_max > lat_min + 1e-4 {
            dist.flags[3] = (lat_max - lat_min) * 0.5;
        }
    });
}

/// Drops CPU mesh/texture RAM after GPU upload (same contract as shop).
pub fn release_hallway_environment_cpu_sources_after_gpu_upload() {
    let mut g = HALLWAY_GLB_CPU.write();
    if let HallwayGlbCache::Ready(Some(cpu)) = &mut *g {
        room_glb::release_room_environment_primitives_cpu(cpu);
    }
}

#[inline]
fn is_hallway_marker_name(name: &str) -> bool {
    matches!(name, BTN_PLAY_ROUND | BTN_PLAY_BOSS | BTN_SKIP_ROUND)
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
    tag_hallway_walls_for_runtime_tint(&mut cpu);
    Ok(cpu)
}

/// World-space marker position (same centering + scale as uploaded hallway mesh).
pub fn hallway_marker_world(
    window_h: f32,
    env_height_scale: f32,
    cpu: &RoomGlbCpu,
    name: &str,
) -> Option<Vec3> {
    let t = room_glb::marker_translation(cpu, name)?;
    let s = room_glb::room_env_world_scale(window_h, env_height_scale);
    Some(t * s)
}

/// First embedded perspective camera (shop preferred name or depth-first fallback).
#[cfg(test)]
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
pub fn hallway_pick_chamber_embedded_camera_params(
    window_h: f32,
    env_height_scale: f32,
    ordeal_chamber: bool,
) -> Option<CameraParams> {
    with_hallway_glb_cpu(|opt| {
        let cpu = opt?;
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        pick_chamber_embedded_camera_doc(cpu, ordeal_chamber)
            .map(|c| c.to_camera_params(window_h, env_height_scale, center_doc))
    })
}

fn hallway_camera_resolve(
    w: f32,
    h: f32,
    env_h: f32,
    from_glb: Option<CameraParams>,
) -> CameraParams {
    with_hallway_glb_cpu(|opt| {
        let mut cam = from_glb.unwrap_or_else(|| CameraParams {
            eye: [0.0, -h * 1.25, h * 0.50],
            target: [0.0, h * 0.05, h * 0.18],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 55.0,
            clip_near: None,
            clip_far: None,
        });
        if from_glb.is_none()
            && let Some(cpu) = opt
        {
            let corners = room_glb::room_world_bounds_corners_centered(h, env_h, cpu);
            cam = room_glb::room_camera_fit_fovy_for_corners(w, h, cam, &corners, 0.94);
        }
        if let Some(cpu) = opt {
            cam = room_glb::room_camera_with_room_clip_planes(cam, h, env_h, cpu);
        }
        cam
    })
}

/// Camera for pick-blind: `default` / `boss` glTF cameras when present, else same as [`hallway_camera_base`].
pub fn hallway_camera_pick_chamber(
    w: f32,
    h: f32,
    env_h: f32,
    ordeal_chamber: bool,
) -> CameraParams {
    // Resolve outside nested `with_hallway_glb_cpu` — avoids deadlock on first load.
    let from_glb = hallway_pick_chamber_embedded_camera_params(h, env_h, ordeal_chamber);
    hallway_camera_resolve(w, h, env_h, from_glb)
}

/// Legacy pick-blind camera (single embedded cam or bounds fit). Unit tests use this; gameplay uses [`hallway_camera_pick_chamber`].
#[cfg(test)]
pub fn hallway_camera_base(w: f32, h: f32, env_h: f32) -> CameraParams {
    let from_glb = hallway_camera_from_glb_if_present(h, env_h);
    hallway_camera_resolve(w, h, env_h, from_glb)
}

pub fn hallway_glb_has_embedded_lights() -> bool {
    with_hallway_glb_cpu(|opt| {
        opt.is_some_and(crate::room_gltf_punctual::room_glb_has_embedded_lights)
    })
}

pub fn hallway_embedded_point_lights_runtime_tagged(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<crate::room_gltf_punctual::EmbeddedPointLightRuntime> {
    with_hallway_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_point_lights_runtime_tagged(
                cpu,
                w,
                h,
                env_h,
                tune,
                crate::room_gltf_punctual::RoomPunctualProfile::Standard,
                "hallway.glb",
            )
        })
        .unwrap_or_default()
    })
}

/// glTF punctual points merged into [`crate::draw_cmd::SceneLighting::punctual`] (hallway room).
pub fn hallway_embedded_point_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<PointLight> {
    hallway_embedded_point_lights_runtime_tagged(w, h, env_h, tune)
        .into_iter()
        .map(|t| t.light)
        .collect()
}

/// glTF spot lights for [`UiFrame::spot_lights`].
pub fn hallway_embedded_spot_lights_runtime(
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
) -> Vec<SpotLight> {
    with_hallway_glb_cpu(|opt| {
        opt.map(|cpu| {
            crate::room_gltf_punctual::embedded_spot_lights_runtime(
                cpu,
                w,
                h,
                env_h,
                tune,
                "hallway.glb",
            )
        })
        .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::load_hallway_glb_from_bytes;
    use mahjuro_core::core::rules::ChamberKind;
    use crate::hallway_glb::{
        HALLWAY_RIPPLE_AMOUNT, HALLWAY_RIPPLE_SPEED, HALLWAY_RIPPLE_WAVES, HallwayDistortion,
    };
    use crate::room_glb;

    #[test]
    fn hallway_env_and_light_node_inventory() {
        let Some(file) = mahjuro_assets::asset_path::get("3d/hallway.glb") else {
            eprintln!("skip: no hallway.glb");
            return;
        };
        let cpu = load_hallway_glb_from_bytes(&file.data).expect("decode");
        eprintln!("=== hallway environment_primitives ({}):", cpu.environment_primitives.len());
        for ep in &cpu.environment_primitives {
            eprintln!("  mesh {:?}", ep.gltf_node_name);
        }
        eprintln!("=== embedded_point_lights ({}):", cpu.embedded_point_lights.len());
        for l in &cpu.embedded_point_lights {
            eprintln!(
                "  {:?} pos_doc={:?}",
                l.node_name, l.pos_doc
            );
        }
        if let Some(b) = cpu.environment_bounds_doc {
            eprintln!("=== bounds min={:?} max={:?}", b.min, b.max);
        }
    }

    #[test]
    fn hallway_wall_tint_tag_does_not_disable_directional_shadows() {
        use crate::room_env_gltf::{
            ROOM_ENV_COLOR_A_ARCHIVE_NO_DIRECTIONAL_SHADOW, ROOM_ENV_COLOR_A_HALLWAY_WALL_TINT,
        };
        assert!(
            (super::HALLWAY_WALL_TINT_COLOR_TAG - ROOM_ENV_COLOR_A_HALLWAY_WALL_TINT).abs()
                < 1e-6
        );
        assert!(
            (super::HALLWAY_WALL_TINT_COLOR_TAG
                - ROOM_ENV_COLOR_A_ARCHIVE_NO_DIRECTIONAL_SHADOW)
                .abs()
                > 0.5,
            "hallway wall tint must not reuse archive no-shadow tag"
        );
    }

    #[test]
    fn hallway_walls_vertices_have_ripple_wall_weight() {
        let Some(file) = mahjuro_assets::asset_path::get("3d/hallway.glb") else {
            eprintln!("skip: no hallway.glb");
            return;
        };
        let cpu = load_hallway_glb_from_bytes(&file.data).expect("decode");
        let walls = cpu
            .environment_primitives
            .iter()
            .find(|ep| ep.gltf_node_name.as_deref() == Some(super::HALLWAY_WALLS_NODE))
            .expect("walls primitive");
        let window_h = 1080.0f32;
        let env_h = 1.0f32;
        let mut dist = HallwayDistortion::from_pick_chamber(ChamberKind::Big, 1, 1, 1);
        super::hallway_distortion_apply_glb_depth_extent(&mut dist, window_h, env_h);
        let m = room_glb::room_env_model_matrix_from_cpu(window_h, env_h, &cpu);
        let axis_doc = super::hallway_depth_axis_doc();
        let axis_n = m.transform_vector3(axis_doc).normalize();
        let mut lateral_n = axis_n.cross(Vec3::Z);
        if lateral_n.length_squared() < 1e-8 {
            lateral_n = Vec3::X;
        } else {
            lateral_n = lateral_n.normalize();
        }
        let lat_half = dist.flags[3].max(0.25);
        let lat_ref = dist.stretch[1];
        let d0 = dist.mask[2];
        let d1 = dist.mask[3];
        let span = (d1 - d0).max(1e-4);
        let ripple_wall_power = 1.28f32;
        let mut min_ripple_wall = f32::INFINITY;
        let mut max_ripple_wall = 0.0f32;
        let mut min_u = f32::INFINITY;
        let mut max_u = 0.0f32;
        for v in &walls.mesh.vertices {
            let p = m.transform_point3(Vec3::from(v.position));
            let side_c = p.dot(lateral_n) - lat_ref;
            let ripple_wall = side_c.abs().max(1e-4).powf(ripple_wall_power);
            min_ripple_wall = min_ripple_wall.min(ripple_wall);
            max_ripple_wall = max_ripple_wall.max(ripple_wall);
            let u = ((p.dot(axis_n) - d0) / span).clamp(0.0, 1.0);
            min_u = min_u.min(u);
            max_u = max_u.max(u);
        }
        eprintln!(
            "walls ripple_wall [{min_ripple_wall:.4}, {max_ripple_wall:.4}] u [{min_u:.4}, {max_u:.4}] lat_half={lat_half}"
        );
        assert!(
            lat_half <= 48.0,
            "flags.w should be p90 |side_c| capped; got {lat_half}"
        );
        assert!(
            max_ripple_wall > 0.5,
            "walls should reach meaningful ripple weight; got max_ripple_wall={max_ripple_wall}"
        );
        assert!(
            max_u - min_u > 0.05,
            "u should vary along corridor on wall mesh; span={}",
            max_u - min_u
        );
        let balloon_amp = dist.bow[3];
        let mut max_bulge = 0.0f32;
        for v in &walls.mesh.vertices {
            let p = m.transform_point3(Vec3::from(v.position));
            let wall_dist = (p.dot(lateral_n) - lat_ref).abs();
            let on_wall = test_smoothstep(0.12, 1.35, wall_dist);
            let u = ((p.dot(axis_n) - d0) / span).clamp(0.0, 1.0);
            let depth_barrel = (u * std::f32::consts::PI).sin().max(0.22);
            let z_mid = (super::HALLWAY_BALLOON_FLOOR_Z + dist.ceiling[1]) * 0.5;
            let z_half = ((dist.ceiling[1] - super::HALLWAY_BALLOON_FLOOR_Z) * 0.5).max(0.18);
            let z_n = ((p.z - z_mid) / z_half).clamp(-1.0, 1.0);
            let vert_barrel = (1.0 - z_n * z_n).max(0.4);
            max_bulge = max_bulge.max(balloon_amp * on_wall * depth_barrel * vert_barrel);
        }
        assert!(
            max_bulge > 0.002,
            "wall barrel bow should displace wall verts; max_bulge={max_bulge} bow.w={balloon_amp}"
        );
    }

    #[inline]
    fn test_smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
        let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    #[test]
    fn hallway_wing_intensity_ramps_to_final_wing() {
        assert!(
            (super::hallway_wing_intensity_scale(1) - super::HALLWAY_WING_INTENSITY_AT_FIRST).abs()
                < 1e-5
        );
        assert!(
            (super::hallway_wing_intensity_scale(super::HALLWAY_WING_FINAL)
                - super::HALLWAY_WING_INTENSITY_AT_FINAL)
                .abs()
                < 1e-5
        );
        assert!(
            (super::hallway_wing_intensity_scale(super::HALLWAY_WING_FINAL + 2)
                - super::hallway_wing_intensity_scale(super::HALLWAY_WING_FINAL))
                .abs()
                < 1e-5
        );
    }

    #[test]
    fn pick_chamber_distortion_populates_lateral_ripple() {
        let d = HallwayDistortion::from_pick_chamber(ChamberKind::Big, 0xA5A5_5A5A_A5A5_5A5Au64, 3, 2);
        assert!(d.ripple[0] > 1e-5, "ripple amplitude");
        assert!(d.ripple[1] >= HALLWAY_RIPPLE_WAVES * 0.89);
        assert!((d.ripple[2] - HALLWAY_RIPPLE_SPEED).abs() < 1e-5);
        assert!(d.ripple[3] >= 0.55 && d.ripple[3] <= 0.85);
        let boss = HallwayDistortion::from_pick_chamber(ChamberKind::Ordeal, 0xA5A5_5A5A_A5A5_5A5Au64, 3, 2);
        assert!(
            boss.ripple[0] >= HALLWAY_RIPPLE_AMOUNT * 0.39,
            "boss ripple keeps a visibility floor"
        );
        assert!(boss.ripple[0] < d.ripple[0]);
    }

    /// Pick-blind room (`hallway.glb`) — documents how many environment primitives carry glTF
    /// emissive; re-run after authoring so the count reflects `emissiveTexture` / factor.
    #[test]
    fn pick_chamber_camera_uses_tight_clip_planes() {
        let w = 1920.0;
        let h = 1080.0;
        let cam = super::hallway_camera_base(w, h, 1.0);
        let (near, far) = cam.clip_planes(h);
        let ratio = far / near;
        let legacy_ratio = (h * crate::draw_cmd::SCENE_PERSPECTIVE_FAR_MUL) / 1.0;
        assert!(
            near >= crate::room_env_gltf::ROOM_CAMERA_CLIP_NEAR_MIN,
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
    fn pick_chamber_hallway_has_walls_primitive() {
        let Some(file) = mahjuro_assets::asset_path::get("3d/hallway.glb") else {
            eprintln!(
                "skip pick_chamber_hallway_has_walls_primitive: no 3d/hallway.glb (bake packs or set MAHJURO_ASSETS)"
            );
            return;
        };
        let cpu = load_hallway_glb_from_bytes(&file.data).expect("hallway.glb decode");
        let walls = cpu
            .environment_primitives
            .iter()
            .find(|ep| ep.gltf_node_name.as_deref() == Some(super::HALLWAY_WALLS_NODE));
        assert!(
            walls.is_some(),
            "hallway.glb must include a mesh node named `{}`",
            super::HALLWAY_WALLS_NODE
        );
        assert!(
            walls
                .unwrap()
                .mesh
                .vertices
                .iter()
                .all(|v| (v.color[3] - super::HALLWAY_WALL_TINT_COLOR_TAG).abs() < 1e-4),
            "walls vertices must carry the hallway wall tint tag"
        );
    }

    #[test]
    fn pick_chamber_room_emissive_material_summary() {
        let data = match mahjuro_assets::asset_path::get("3d/hallway.glb") {
            Some(f) => f.data,
            None => {
                eprintln!(
                    "skip pick_chamber_room_emissive_material_summary: no 3d/hallway.glb (bake packs or set MAHJURO_ASSETS)"
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
    fn pick_chamber_hallway_boss_camera_and_play_boss_marker() {
        let Some(file) = mahjuro_assets::asset_path::get("3d/hallway.glb") else {
            eprintln!(
                "skip pick_chamber_hallway_boss_camera_and_play_boss_marker: no 3d/hallway.glb (bake packs or set MAHJURO_ASSETS)"
            );
            return;
        };
        let cpu = load_hallway_glb_from_bytes(&file.data).expect("hallway.glb decode");
        let by = &cpu.embedded_cameras_by_name;
        if !by.contains_key("default") || !by.contains_key("boss") {
            let keys: Vec<&String> = by.keys().collect();
            eprintln!(
                "skip pick_chamber_hallway_boss_camera_and_play_boss_marker: need perspective camera nodes named `default` and `boss` (got {} named camera(s): {:?})",
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
            "hallway `default` and `boss` cameras should differ in eye/target for pick_chamber; got eye_sum_abs_diff={diff_eye} target_sum_abs_diff={diff_tgt}"
        );

        let has_boss_play = room_glb::marker_translation(&cpu, super::BTN_PLAY_BOSS).is_some()
            || cpu
                .marker_mesh_bounds_doc_for(super::BTN_PLAY_BOSS)
                .is_some();
        if !has_boss_play {
            eprintln!(
                "skip pick_chamber_hallway_boss_camera_and_play_boss_marker: add `btn_play_boss` (transform and/or mesh) for boss pick-blind play hit target"
            );
            return;
        }

        let cam_pb_false = super::hallway_camera_pick_chamber(1920.0, 1080.0, env_h, false);
        let cam_pb_true = super::hallway_camera_pick_chamber(1920.0, 1080.0, env_h, true);
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
            "hallway_camera_pick_chamber(boss=false) vs boss=true should differ when default/boss cameras differ; got combined_abs_diff={d}"
        );
    }
}
