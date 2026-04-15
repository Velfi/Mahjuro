//! Floating 3D score popups used for short-lived textual beats such as
//! zodiac level-ups or generic structure-growth callouts.

use std::time::Instant;

use rand::RngExt;

use crate::core::scoring::StepKind;
use crate::render::draw_cmd::ExtrudedGlyphPlacement;

/// Total lifetime of a popup from spawn to despawn (seconds).
const LIFETIME: f32 = 1.4;

/// Phase boundaries on the [0, 1] normalised lifetime axis.
const T_BIRTH_END: f32 = 0.15;
const T_SETTLE_END: f32 = 0.35;
const T_DRIFT_END: f32 = 0.85;

/// Slight per-popup yaw jitter so a chain of popups doesn't read as a
/// stamped row of identical objects.
const YAW_JITTER: f32 = 0.07;

/// World-units height the popup floats above the table plane while in
/// flight. The settle phase drifts this slightly upward; the drift phase
/// keeps it constant.
const LIFT_BASE: f32 = 450.0;
const LIFT_DRIFT: f32 = 120.0;

#[derive(Clone, Debug)]
struct ScorePopup {
    label: String,
    born_at: Instant,
    source_xy: (f32, f32),
    dest_xy: (f32, f32),
    base_scale: f32,
    color: [f32; 4],
    yaw: f32,
    motion: PopupMotion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopupMotion {
    Drift,
    Shake,
}

pub struct ScorePopupSystem {
    popups: Vec<ScorePopup>,
}

impl ScorePopupSystem {
    pub fn new() -> Self {
        Self { popups: Vec::new() }
    }

    /// Spawn a new popup. `magnitude` scales the object so more important
    /// beats can land with a little more visual weight.
    pub fn spawn(
        &mut self,
        label: String,
        source_xy: (f32, f32),
        dest_xy: (f32, f32),
        kind: StepKind,
        magnitude: f32,
    ) {
        let color = kind_color(kind);
        let mag = magnitude.abs().max(1.0);
        let scale = 205.0 * (1.0 + (mag.log2() / 12.0).clamp(0.0, 0.42));
        let mut rng = rand::rng();
        let yaw = (rng.random::<f32>() - 0.5) * YAW_JITTER;
        self.popups.push(ScorePopup {
            label,
            born_at: Instant::now(),
            source_xy,
            dest_xy,
            base_scale: scale,
            color,
            yaw,
            motion: PopupMotion::Drift,
        });
    }

    /// Spawn a red warning X that shakes in place over a debuffed scorer as
    /// it is being evaluated by the cascade.
    pub fn spawn_debuff_x(&mut self, source_xy: (f32, f32), magnitude: f32) {
        let mag = magnitude.abs().max(1.0);
        let scale = 180.0 * (1.0 + (mag.log2() / 10.0).clamp(0.0, 0.35));
        let mut rng = rand::rng();
        let yaw = (rng.random::<f32>() - 0.5) * (YAW_JITTER * 0.6);
        self.popups.push(ScorePopup {
            label: "X".to_string(),
            born_at: Instant::now(),
            source_xy,
            dest_xy: source_xy,
            base_scale: scale,
            color: [0.96, 0.24, 0.20, 1.0],
            yaw,
            motion: PopupMotion::Shake,
        });
    }

    /// Advance the system; despawn any popups whose lifetime has elapsed.
    pub fn update(&mut self, now: Instant) {
        self.popups
            .retain(|p| now.saturating_duration_since(p.born_at).as_secs_f32() < LIFETIME);
    }

    /// Drop every popup immediately.
    pub fn clear(&mut self) {
        self.popups.clear();
    }

    pub fn is_active(&self) -> bool {
        !self.popups.is_empty()
    }

    /// Build the per-frame placement list the renderer consumes.
    pub fn placements(&self, now: Instant) -> Vec<ExtrudedGlyphPlacement> {
        self.popups
            .iter()
            .map(|p| {
                let age = now.saturating_duration_since(p.born_at).as_secs_f32();
                let t = (age / LIFETIME).clamp(0.0, 1.0);

                // ── Lifecycle phases ───────────────────────────────────
                let (scale_mul, alpha, pos_t, lift_extra) = if t < T_BIRTH_END {
                    let local = t / T_BIRTH_END;
                    let s = (local * std::f32::consts::FRAC_PI_2).sin() * 1.10;
                    (s, 1.0, 0.0, 0.0)
                } else if t < T_SETTLE_END {
                    let local = (t - T_BIRTH_END) / (T_SETTLE_END - T_BIRTH_END);
                    let s = 1.10 + (1.0 - 1.10) * local;
                    (s, 1.0, 0.0, LIFT_DRIFT * local)
                } else if t < T_DRIFT_END {
                    let local = (t - T_SETTLE_END) / (T_DRIFT_END - T_SETTLE_END);
                    let pt = local * local;
                    (1.0, 1.0, pt, LIFT_DRIFT)
                } else {
                    let local = (t - T_DRIFT_END) / (1.0 - T_DRIFT_END);
                    let s = (1.0 - local).max(0.0);
                    let a = (1.0 - local).max(0.0);
                    (s, a, 1.0, LIFT_DRIFT * (1.0 - local))
                };

                let mut px = p.source_xy.0 + (p.dest_xy.0 - p.source_xy.0) * pos_t;
                let mut py = p.source_xy.1 + (p.dest_xy.1 - p.source_xy.1) * pos_t;
                if p.motion == PopupMotion::Shake {
                    let env = if t < 0.8 { 1.0 - t / 0.8 } else { 0.0 };
                    let shake = env * p.base_scale * 0.05;
                    px += (t * 55.0).sin() * shake;
                    py += (t * 39.0).cos() * shake * 0.55;
                }
                let mut color = p.color;
                color[3] *= alpha;

                let emissive = if p.motion == PopupMotion::Shake {
                    0.9
                } else if t < T_BIRTH_END {
                    1.0
                } else if t < T_SETTLE_END {
                    1.0 - (t - T_BIRTH_END) / (T_SETTLE_END - T_BIRTH_END)
                } else {
                    0.2
                };

                ExtrudedGlyphPlacement {
                    world_pos: [px, py, LIFT_BASE + lift_extra],
                    scale: p.base_scale * scale_mul,
                    rotation_x: 0.08,
                    rotation_y: p.yaw,
                    label: p.label.clone(),
                    color,
                    emissive,
                }
            })
            .collect()
    }
}

fn kind_color(kind: StepKind) -> [f32; 4] {
    match kind {
        StepKind::Chips => [0.94, 0.86, 0.60, 1.0],
        StepKind::Mult => [0.98, 0.90, 0.68, 1.0],
        StepKind::Gold => [1.00, 0.91, 0.66, 1.0],
        StepKind::Final => [1.00, 0.95, 0.76, 1.0],
    }
}

impl Default for ScorePopupSystem {
    fn default() -> Self {
        Self::new()
    }
}
