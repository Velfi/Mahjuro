//! Floating 3D score popups: per-step "+50" / "×3" / "=12500" labels that
//! pop in at the source of a scoring contribution, settle, drift toward the
//! score panel, then collapse into it.
//!
//! Modeled on `falling_bones.rs`: a simple `Vec<ScorePopup>` with a
//! per-frame `update(dt)` and a `placements()` accessor that the renderer
//! consumes via `DrawCmd::ExtrudedGlyphBatch`. The system itself doesn't
//! care about font, mesh tessellation, or GPU upload — it just emits
//! `ExtrudedGlyphPlacement`s with the desired pose and tint each frame and
//! lets the renderer's lazy `GlyphMeshCache` build the matching meshes.

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
const YAW_JITTER: f32 = 0.18;

/// World-units height the popup floats above the table plane while in
/// flight. The settle phase drifts this slightly upward; the drift phase
/// keeps it constant.
const LIFT_BASE: f32 = 90.0;
const LIFT_DRIFT: f32 = 30.0;

#[derive(Clone, Debug)]
struct ScorePopup {
    label: String,
    born_at: Instant,
    source_xy: (f32, f32),
    dest_xy: (f32, f32),
    base_scale: f32,
    color: [f32; 4],
    yaw: f32,
}

pub struct ScorePopupSystem {
    popups: Vec<ScorePopup>,
}

impl ScorePopupSystem {
    pub fn new() -> Self {
        Self { popups: Vec::new() }
    }

    /// Spawn a new popup. `magnitude` is the absolute numeric delta the
    /// label represents (e.g. 50 for "+50", 3 for "×3"); it scales the
    /// popup so big numbers are visibly bigger objects on screen.
    pub fn spawn(
        &mut self,
        label: String,
        source_xy: (f32, f32),
        dest_xy: (f32, f32),
        kind: StepKind,
        magnitude: f32,
    ) {
        // Tint by axis. Chips read cool indigo, mult warm crimson, the final
        // beat champagne gold — same colour grammar the rest of the cascade
        // HUD already uses.
        let color = match kind {
            StepKind::Chips => [0.62, 0.78, 1.0, 1.0],
            StepKind::Mult => [1.0, 0.55, 0.55, 1.0],
            StepKind::Final => [1.0, 0.92, 0.45, 1.0],
        };
        // Magnitude → scale: log curve so a +5 popup is still readable but
        // a +500 popup is dramatically bigger. Base height ≈ 60 world units
        // (the glyph mesh is normalised to height 1.0, so this is the
        // direct world-space character height).
        let mag = magnitude.abs().max(1.0);
        let scale = 60.0 * (1.0 + (mag.log2() / 8.0).clamp(0.0, 0.8));
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
        });
    }

    /// Advance the system; despawn any popups whose lifetime has elapsed.
    pub fn update(&mut self, now: Instant) {
        self.popups.retain(|p| {
            now.saturating_duration_since(p.born_at).as_secs_f32() < LIFETIME
        });
    }

    /// Drop every popup immediately. Called when the cascade ends or is
    /// skipped so the play space clears for the next hand.
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
                // 0..T_BIRTH_END  — pop in (0 → 1.25× base) at source_xy
                // T_BIRTH_END..T_SETTLE_END — settle to base, drift up
                // T_SETTLE_END..T_DRIFT_END — fly toward dest, hold scale
                // T_DRIFT_END..1.0 — collapse (scale + alpha → 0)
                let (scale_mul, alpha, pos_t, lift_extra) = if t < T_BIRTH_END {
                    let local = t / T_BIRTH_END;
                    // Ease-out-back overshoot: 1 + (1.70158+1)*(local-1)^3
                    // simplified to a quick 0 → 1.25 lift via sine.
                    let s = (local * std::f32::consts::FRAC_PI_2).sin() * 1.25;
                    (s, 1.0, 0.0, 0.0)
                } else if t < T_SETTLE_END {
                    let local =
                        (t - T_BIRTH_END) / (T_SETTLE_END - T_BIRTH_END);
                    // Relax 1.25 → 1.0
                    let s = 1.25 + (1.0 - 1.25) * local;
                    (s, 1.0, 0.0, LIFT_DRIFT * local)
                } else if t < T_DRIFT_END {
                    let local =
                        (t - T_SETTLE_END) / (T_DRIFT_END - T_SETTLE_END);
                    // Ease-in-cubic: accelerates toward the score panel.
                    let pt = local * local * local;
                    (1.0, 1.0, pt, LIFT_DRIFT)
                } else {
                    let local = (t - T_DRIFT_END) / (1.0 - T_DRIFT_END);
                    let s = (1.0 - local).max(0.0);
                    let a = (1.0 - local).max(0.0);
                    (s, a, 1.0, LIFT_DRIFT * (1.0 - local))
                };

                let px = p.source_xy.0 + (p.dest_xy.0 - p.source_xy.0) * pos_t;
                let py = p.source_xy.1 + (p.dest_xy.1 - p.source_xy.1) * pos_t;
                let mut color = p.color;
                color[3] *= alpha;

                // Light emissive ramps with the birth pop and decays as the
                // popup drifts so the freshly-spawned label catches the eye.
                let emissive = if t < T_BIRTH_END {
                    1.0
                } else if t < T_SETTLE_END {
                    1.0 - (t - T_BIRTH_END) / (T_SETTLE_END - T_BIRTH_END)
                } else {
                    0.2
                };

                ExtrudedGlyphPlacement {
                    world_pos: [px, py, LIFT_BASE + lift_extra],
                    scale: p.base_scale * scale_mul,
                    rotation_x: 0.18, // gentle rake toward the camera
                    rotation_y: p.yaw,
                    label: p.label.clone(),
                    color,
                    emissive,
                }
            })
            .collect()
    }
}

impl Default for ScorePopupSystem {
    fn default() -> Self {
        Self::new()
    }
}
