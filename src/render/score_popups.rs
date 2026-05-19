//! Floating score popups used for short-lived textual beats such as zodiac
//! level-ups or generic structure-growth callouts.
//!
//! [`ScorePopupSystem::placements`] feeds extruded 3D glyphs (gameplay). The
//! shop uses [`ScorePopupSystem::overlay_text_labels`] so celebrations draw in
//! the text-overlay pass above the 3D scene.

use std::sync::Arc;
use std::time::Instant;

use rand::RngExt;

use crate::core::scoring::StepKind;
use crate::render::draw_cmd::{GlyphMaterial, Object3d, Object3dKind};
use crate::render::theme::typography;
use crate::render::wgpu_renderer::{TextAlign, TextLabel};

/// Total lifetime of a popup from spawn to despawn (seconds).
const LIFETIME: f32 = 1.18;

/// Phase boundaries on the [0, 1] normalised lifetime axis. Streaming popups
/// spend a brief moment at their source (birth + hover), then arc toward the
/// score reel and shrink into it during the stream phase.
const T_BIRTH_END: f32 = 0.08;
const T_HOVER_END: f32 = 0.36;

/// Slight per-popup yaw jitter so a chain of popups doesn't read as a
/// stamped row of identical objects.
const YAW_JITTER: f32 = 0.07;

/// World-units height the popup floats above the table plane while in
/// flight. The hover phase drifts this slightly upward; the stream phase
/// arcs higher still via the bezier control point.
const LIFT_BASE: f32 = 450.0;
const LIFT_HOVER: f32 = 135.0;
/// Extra lift applied at the midpoint of the streaming arc. Drives the
/// "over the top" feel of popups flying into the reel. Kept modest so
/// the arc stays within the camera frustum across screen sizes.
const LIFT_ARC_PEAK: f32 = 148.0;

/// Sky-blue base tint for Chips popups. The Polychrome shader adds a
/// rainbow thin-film sheen on top of this as the light sweeps across.
const CHIPS_COLOR: [f32; 4] = crate::render::theme::color::LAPIS;
/// Red base tint for Mult popups.
const MULT_COLOR: [f32; 4] = crate::render::theme::color::RUBY;
/// Warm gold base tint for Gold popups.
const GOLD_COLOR: [f32; 4] = crate::render::theme::color::RELIC_GOLD;
/// Cream tint for the Final landing number.
const FINAL_COLOR: [f32; 4] = crate::render::theme::color::TALLOW;

#[derive(Clone, Debug)]
struct ScorePopup {
    label: Arc<str>,
    born_at: Instant,
    source_xy: (f32, f32),
    dest_xy: (f32, f32),
    /// World-Z lift the stream phase eases toward at landing. Source lift
    /// is always `LIFT_BASE`; the stream sample interpolates up to this so
    /// popups land *at* the reel's depth instead of on the table plane.
    dest_lift: f32,
    base_scale: f32,
    color: [f32; 4],
    yaw: f32,
    motion: PopupMotion,
    material: GlyphMaterial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopupMotion {
    /// Stream from source to dest along a lifted bezier arc, shrinking to
    /// zero as it lands in the reel. Used for Chips / Mult / Gold.
    Stream,
    /// Stay at source, fade out in place. Used for the Final landing beat
    /// (which already spawns at the reel).
    Settle,
    /// Shake in place, then fade. Used for debuff X warnings.
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
    /// beats can land with a little more visual weight. `dest_lift` is the
    /// world-Z the streaming phase eases toward — pass the reel's lift so
    /// popups land *at* the reel's depth rather than on the table plane.
    /// `None` falls back to `LIFT_BASE` (the old behavior).
    pub fn spawn(
        &mut self,
        label: impl Into<Arc<str>>,
        source_xy: (f32, f32),
        dest_xy: (f32, f32),
        dest_lift: Option<f32>,
        kind: StepKind,
        magnitude: f32,
    ) {
        let (color, material, motion) = match kind {
            StepKind::Chips => (CHIPS_COLOR, GlyphMaterial::Polychrome, PopupMotion::Stream),
            StepKind::Mult => (MULT_COLOR, GlyphMaterial::Polychrome, PopupMotion::Stream),
            StepKind::Gold => (GOLD_COLOR, GlyphMaterial::Polychrome, PopupMotion::Stream),
            StepKind::Final => (FINAL_COLOR, GlyphMaterial::Polychrome, PopupMotion::Settle),
        };
        let mag = magnitude.abs().max(1.0);
        let scale = 198.0 * (1.0 + (mag.log2() / 12.0).clamp(0.0, 0.48));
        let mut rng = rand::rng();
        let yaw = (rng.random::<f32>() - 0.5) * YAW_JITTER;
        self.popups.push(ScorePopup {
            label: label.into(),
            born_at: Instant::now(),
            source_xy,
            dest_xy,
            dest_lift: dest_lift.unwrap_or(LIFT_BASE),
            base_scale: scale,
            color,
            yaw,
            motion,
            material,
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
            label: Arc::from("X"),
            born_at: Instant::now(),
            source_xy,
            dest_xy: source_xy,
            dest_lift: LIFT_BASE,
            base_scale: scale,
            color: [0.96, 0.24, 0.20, 1.0],
            yaw,
            motion: PopupMotion::Shake,
            material: GlyphMaterial::Polychrome,
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

    /// Screen-space labels with the same timing as [`Self::placements`], for
    /// the text-overlay pass (depth test Always) so shop celebrations read
    /// above counter geometry.
    pub fn overlay_text_labels(
        &self,
        now: Instant,
        w: f32,
        h: f32,
        screen_scale: f32,
    ) -> Vec<TextLabel> {
        self.popups
            .iter()
            .map(|p| {
                let (_px, py, _lift_z, scale_mul, alpha, _emissive) = popup_frame_sample(p, now);
                let mut color = p.color;
                color[3] *= alpha;
                let font_px = typography::tier_at_most(
                    (p.base_scale * scale_mul * screen_scale * 0.38).min(h * 0.14),
                    h,
                );
                let line_h = font_px * 1.35;
                let top = (py - line_h * 0.5).clamp(0.0, (h - line_h).max(0.0));
                TextLabel {
                    text: p.label.to_string(),
                    rect: [0.0, top, w, line_h],
                    color,
                    font_px: Some(font_px),
                    align: TextAlign::Center,
                    no_glossary: true,
                    ..Default::default()
                }
            })
            .collect()
    }

    /// Build the per-frame placement list the renderer consumes.
    ///
    /// `screen_scale` multiplies each popup's world-units scale so they stay
    /// readable at the current window size. Callers derive this from
    /// `min(window_w, window_h) / 1080.0` so small windows get proportionally
    /// smaller popups.
    pub fn placements(&self, now: Instant, screen_scale: f32) -> Vec<Object3d> {
        self.popups
            .iter()
            .map(|p| {
                let (px, py, lift_z, scale_mul, alpha, emissive) = popup_frame_sample(p, now);

                let mut color = p.color;
                color[3] *= alpha;

                Object3d {
                    pos: [px, py, lift_z],
                    extents: [1.0, 1.0, 1.0],
                    rotation: [0.0, 0.0, 0.0],
                    color,
                    kind: Object3dKind::ExtrudedGlyph {
                        scale: p.base_scale * scale_mul * screen_scale,
                        rotation_x: 0.08,
                        rotation_y: p.yaw,
                        // Arc clone — refcount bump, no allocation.
                        label: Arc::clone(&p.label),
                        emissive,
                        material: p.material,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                }
            })
            .collect()
    }
}

fn popup_frame_sample(p: &ScorePopup, now: Instant) -> (f32, f32, f32, f32, f32, f32) {
    let age = now.saturating_duration_since(p.born_at).as_secs_f32();
    let t = (age / LIFETIME).clamp(0.0, 1.0);
    match p.motion {
        PopupMotion::Stream => stream_sample(p, t),
        PopupMotion::Settle => settle_sample(p, t),
        PopupMotion::Shake => shake_sample(p, t),
    }
}

/// Streaming trajectory: brief birth/hover at source, then a lifted bezier
/// arc into the destination, shrinking to zero as the popup lands in the
/// reel. No separate fade phase — absorption into the reel replaces it.
///
/// Return tuple: `(world_x, world_y, world_z_lift, scale_mul, alpha, emissive)`.
fn stream_sample(p: &ScorePopup, t: f32) -> (f32, f32, f32, f32, f32, f32) {
    if t < T_BIRTH_END {
        let local = t / T_BIRTH_END;
        let s = (local * std::f32::consts::FRAC_PI_2).sin() * 1.22;
        return (p.source_xy.0, p.source_xy.1, LIFT_BASE, s, 1.0, 1.05);
    }
    if t < T_HOVER_END {
        let local = (t - T_BIRTH_END) / (T_HOVER_END - T_BIRTH_END);
        let s = 1.22 + (1.0 - 1.22) * local;
        let em = 1.05 - local * 0.55;
        return (
            p.source_xy.0,
            p.source_xy.1,
            LIFT_BASE + LIFT_HOVER * local,
            s,
            1.0,
            em,
        );
    }
    // Stream phase: t ∈ [T_HOVER_END, 1.0]
    let local = (t - T_HOVER_END) / (1.0 - T_HOVER_END);
    // Ease-in-out cubic so the popup accelerates off the source and
    // decelerates as it nears the reel.
    let eased = if local < 0.5 {
        4.0 * local * local * local
    } else {
        1.0 - (-2.0 * local + 2.0).powi(3) * 0.5
    };
    // Quadratic bezier control point: biased slightly toward the destination
    // so the arc reads as deliberately feeding the counter, not a symmetric hump.
    let dx = p.dest_xy.0 - p.source_xy.0;
    let dy = p.dest_xy.1 - p.source_xy.1;
    let ctrl_x = (p.source_xy.0 + p.dest_xy.0) * 0.5 + dx * 0.11;
    let ctrl_y = (p.source_xy.1 + p.dest_xy.1) * 0.5 + dy * 0.14;
    let one_m = 1.0 - eased;
    let px =
        one_m * one_m * p.source_xy.0 + 2.0 * one_m * eased * ctrl_x + eased * eased * p.dest_xy.0;
    let py =
        one_m * one_m * p.source_xy.1 + 2.0 * one_m * eased * ctrl_y + eased * eased * p.dest_xy.1;
    // Ease Z from (LIFT_BASE + LIFT_HOVER) at the start of the stream phase
    // to `dest_lift` at landing, so the popup actually meets the reel's
    // depth. The bezier arc adds an extra peak on top, via the one_m*eased
    // envelope (peaks at 0.25 when eased == 0.5 → scale by 4.0).
    let start_lift = LIFT_BASE + LIFT_HOVER;
    let lerp_lift = start_lift + (p.dest_lift - start_lift) * eased;
    let arc_env = 4.0 * one_m * eased;
    let lift_z = lerp_lift + LIFT_ARC_PEAK * arc_env;
    // Shrink to zero over the last 32% so the popup "lands into" the reel.
    let shrink = if local < 0.68 {
        1.0
    } else {
        ((1.0 - local) / 0.32).max(0.0)
    };
    // Emissive peaks mid-stream (thin-film read) then eases down into landing.
    let stream_glow = 4.0 * one_m * eased;
    let emissive = 0.48 + 0.52 * stream_glow;
    (px, py, lift_z, shrink, 1.0, emissive)
}

/// In-place settle with fade-out. Used by the Final landing number.
fn settle_sample(p: &ScorePopup, t: f32) -> (f32, f32, f32, f32, f32, f32) {
    let (px, py) = p.source_xy;
    if t < T_BIRTH_END {
        let local = t / T_BIRTH_END;
        let s = (local * std::f32::consts::FRAC_PI_2).sin() * 1.10;
        return (px, py, LIFT_BASE, s, 1.0, 1.0);
    }
    let local = (t - T_BIRTH_END) / (1.0 - T_BIRTH_END);
    let fade = if local < 0.6 {
        1.0
    } else {
        ((1.0 - local) / 0.4).max(0.0)
    };
    let s = 1.10 + (1.0 - 1.10) * local.min(0.3) / 0.3;
    let em = 1.0 - local.min(0.3) / 0.3 * 0.75;
    (px, py, LIFT_BASE + LIFT_HOVER, s * fade.max(0.01), fade, em)
}

/// Shake-in-place for debuff X warnings.
fn shake_sample(p: &ScorePopup, t: f32) -> (f32, f32, f32, f32, f32, f32) {
    let env = if t < 0.8 { 1.0 - t / 0.8 } else { 0.0 };
    let shake = env * p.base_scale * 0.05;
    let px = p.source_xy.0 + (t * 55.0).sin() * shake;
    let py = p.source_xy.1 + (t * 39.0).cos() * shake * 0.55;
    let (scale_mul, alpha) = if t < T_BIRTH_END {
        let local = t / T_BIRTH_END;
        ((local * std::f32::consts::FRAC_PI_2).sin() * 1.10, 1.0)
    } else if t < 0.85 {
        (1.0, 1.0)
    } else {
        let local = (t - 0.85) / 0.15;
        ((1.0 - local).max(0.0), (1.0 - local).max(0.0))
    };
    (px, py, LIFT_BASE, scale_mul, alpha, 0.9)
}

impl Default for ScorePopupSystem {
    fn default() -> Self {
        Self::new()
    }
}
