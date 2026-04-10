//! Floating 3D score popups: per-step "+50" / "×3" labels that pop in at the
//! source of a scoring contribution, settle, drift toward a floating
//! accumulator label, then collapse into it.
//!
//! The accumulator labels ("120", "×4") persist through the cascade, sitting
//! at a fixed screen position and ticking up as pops land. When the cascade
//! ends the caller calls `clear()` to wipe everything.

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
const LIFT_BASE: f32 = 450.0;
const LIFT_DRIFT: f32 = 30.0;

/// Accumulator label lifetime phases.
const ACCUM_BIRTH_SECS: f32 = 0.20;

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

/// A persistent floating label that accumulates chip or mult totals.
#[derive(Clone, Debug)]
struct Accumulator {
    kind: StepKind,
    pos: (f32, f32),
    /// Current displayed value (ticks up as pops land).
    value_label: String,
    born_at: Instant,
    /// Timestamp of the last value bump — drives a scale pulse.
    last_bump: Instant,
    color: [f32; 4],
    base_scale: f32,
}

pub struct ScorePopupSystem {
    popups: Vec<ScorePopup>,
    accumulators: Vec<Accumulator>,
}

impl ScorePopupSystem {
    pub fn new() -> Self {
        Self {
            popups: Vec::new(),
            accumulators: Vec::new(),
        }
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
        let color = kind_color(kind);
        let mag = magnitude.abs().max(1.0);
        let scale = 300.0 * (1.0 + (mag.log2() / 8.0).clamp(0.0, 0.8));
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

    /// Set (or create) an accumulator for the given axis. The accumulator
    /// sits at `pos` and displays `value_label`. Called each cascade step
    /// so the displayed number tracks the running total.
    pub fn set_accumulator(&mut self, kind: StepKind, pos: (f32, f32), value_label: String) {
        let now = Instant::now();
        if let Some(acc) = self.accumulators.iter_mut().find(|a| a.kind == kind) {
            acc.value_label = value_label;
            acc.last_bump = now;
            acc.pos = pos;
        } else {
            self.accumulators.push(Accumulator {
                kind,
                pos,
                value_label,
                born_at: now,
                last_bump: now,
                color: kind_color(kind),
                base_scale: 380.0,
            });
        }
    }

    /// Advance the system; despawn any popups whose lifetime has elapsed.
    pub fn update(&mut self, now: Instant) {
        self.popups
            .retain(|p| now.saturating_duration_since(p.born_at).as_secs_f32() < LIFETIME);
    }

    /// Drop every popup and accumulator immediately. Called when the cascade
    /// ends or is skipped so the play space clears for the next hand.
    pub fn clear(&mut self) {
        self.popups.clear();
        self.accumulators.clear();
    }

    pub fn is_active(&self) -> bool {
        !self.popups.is_empty() || !self.accumulators.is_empty()
    }

    /// Build the per-frame placement list the renderer consumes.
    pub fn placements(&self, now: Instant) -> Vec<ExtrudedGlyphPlacement> {
        let mut out: Vec<ExtrudedGlyphPlacement> = self
            .popups
            .iter()
            .map(|p| {
                let age = now.saturating_duration_since(p.born_at).as_secs_f32();
                let t = (age / LIFETIME).clamp(0.0, 1.0);

                // ── Lifecycle phases ───────────────────────────────────
                let (scale_mul, alpha, pos_t, lift_extra) = if t < T_BIRTH_END {
                    let local = t / T_BIRTH_END;
                    let s = (local * std::f32::consts::FRAC_PI_2).sin() * 1.25;
                    (s, 1.0, 0.0, 0.0)
                } else if t < T_SETTLE_END {
                    let local = (t - T_BIRTH_END) / (T_SETTLE_END - T_BIRTH_END);
                    let s = 1.25 + (1.0 - 1.25) * local;
                    (s, 1.0, 0.0, LIFT_DRIFT * local)
                } else if t < T_DRIFT_END {
                    let local = (t - T_SETTLE_END) / (T_DRIFT_END - T_SETTLE_END);
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
                    rotation_x: 0.18,
                    rotation_y: p.yaw,
                    label: p.label.clone(),
                    color,
                    emissive,
                }
            })
            .collect();

        // ── Accumulator labels ────────────────────────────────────────────
        for acc in &self.accumulators {
            let age = now.saturating_duration_since(acc.born_at).as_secs_f32();
            let bump_age = now.saturating_duration_since(acc.last_bump).as_secs_f32();

            // Birth: scale from 0 → 1 over ACCUM_BIRTH_SECS.
            let birth_scale = if age < ACCUM_BIRTH_SECS {
                let t = age / ACCUM_BIRTH_SECS;
                (t * std::f32::consts::FRAC_PI_2).sin()
            } else {
                1.0
            };

            // Bump pulse: 1.0 → 1.18 → 1.0 over 0.15s after each value change.
            let bump_scale = if bump_age < 0.15 {
                let t = bump_age / 0.15;
                1.0 + 0.18 * (1.0 - t) * (1.0 - t)
            } else {
                1.0
            };

            // Emissive flash on bump.
            let emissive = if bump_age < 0.12 {
                0.8
            } else if bump_age < 0.3 {
                0.8 * (1.0 - (bump_age - 0.12) / 0.18)
            } else {
                0.15
            };

            out.push(ExtrudedGlyphPlacement {
                world_pos: [acc.pos.0, acc.pos.1, LIFT_BASE + 10.0],
                scale: acc.base_scale * birth_scale * bump_scale,
                rotation_x: 0.18,
                rotation_y: 0.0,
                label: acc.value_label.clone(),
                color: acc.color,
                emissive,
            });
        }

        out
    }
}

fn kind_color(kind: StepKind) -> [f32; 4] {
    match kind {
        StepKind::Chips => [0.62, 0.78, 1.0, 1.0],
        StepKind::Mult => [1.0, 0.55, 0.55, 1.0],
        StepKind::Gold => [0.95, 0.78, 0.25, 1.0],
        StepKind::Final => [1.0, 0.92, 0.45, 1.0],
    }
}

impl Default for ScorePopupSystem {
    fn default() -> Self {
        Self::new()
    }
}
