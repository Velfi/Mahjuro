//! Floating score popups used for short-lived textual beats such as zodiac
//! level-ups or generic structure-growth callouts.
//!
//! [`ScorePopupSystem::placements`] feeds extruded 3D glyphs (gameplay). The
//! shop uses [`ScorePopupSystem::overlay_text_labels`] so celebrations draw in
//! the text-overlay pass above the 3D scene.

use std::sync::Arc;
use std::time::Instant;

use rand::RngExt;

use crate::draw_cmd::{GlyphMaterial, Object3d, Object3dKind};
use crate::theme::typography;
use crate::wgpu_renderer::{TextAlign, TextLabel};
use crate::world_space::LayoutAnchorPx;
use mahjuro_core::core::scoring::StepKind;

/// Per-popup motion timing (seconds + overshoot fraction).
#[derive(Clone, Copy, Debug)]
pub struct PopupMotionTiming {
    pub pop_secs: f32,
    pub loiter_secs: f32,
    pub fly_secs: f32,
    pub overshoot: f32,
}

impl PopupMotionTiming {
    pub fn lifetime_secs(self) -> f32 {
        self.pop_secs + self.loiter_secs + self.fly_secs
    }

    pub fn shipping_default() -> Self {
        Self {
            pop_secs: 0.14,
            loiter_secs: 0.32,
            fly_secs: 0.92,
            overshoot: 0.22,
        }
    }
}

/// Slight per-popup yaw jitter so a chain of popups doesn't read as a
/// stamped row of identical objects.
const YAW_JITTER: f32 = 0.07;

/// World-units height for modifier-strip / screen-layout popup sources.
const LIFT_BASE: f32 = 450.0;
/// Extra lift applied at the midpoint of the streaming arc.
const LIFT_ARC_PEAK: f32 = 148.0;
/// Lift above object-surface anchors (tiles, relics, yaku tablets) so the
/// extruded glyph clears geometry during pop + loiter.
const OBJECT_POPUP_CLEARANCE_SCALE: f32 = 0.58;
const OBJECT_POPUP_CLEARANCE_BASE: f32 = 18.0;

/// Table-hover Z for modifier-strip / screen-layout popup sources (no 3D object).
pub const TABLE_POPUP_LIFT_Z: f32 = LIFT_BASE;

/// **The House** crimson base for structure callouts — polychrome band sweep
/// matches glossary / cash-in ordeal glyphs (`score_glyph_band_albedo` in lit_mesh).
const STRUCTURE_CALLOUT_COLOR: [f32; 4] = crate::theme::color::keyword::HOUSE;
/// Vivid scarlet for Mult score pops (#ff0034).
const HAN_COLOR: [f32; 4] = [1.0, 0.0, 52.0 / 255.0, 1.0];
/// Warm gold base tint for Yen popups.
const GOLD_COLOR: [f32; 4] = crate::theme::color::RELIC_GOLD;
/// Cream tint for the Final landing number.
const FINAL_COLOR: [f32; 4] = crate::theme::color::score_cascade::FINAL;

#[derive(Clone, Debug)]
struct ScorePopup {
    label: Arc<str>,
    born_at: Instant,
    /// Object3d anchor triple at the scoring object's median center.
    source_pos: [f32; 3],
    dest_xy: (f32, f32),
    dest_lift: f32,
    base_scale: f32,
    color: [f32; 4],
    yaw: f32,
    motion: PopupMotion,
    material: GlyphMaterial,
    timing: PopupMotionTiming,
    /// Brighter extruded-glyph halo (structure growth / capacity callouts).
    boost_polychrome_halo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopupMotion {
    /// Pop at source, loiter, then fly into the score roller.
    FlyToReel,
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

    /// Structure growth / capacity callout — **The House** polychrome band sweep.
    pub fn spawn_structure_callout(
        &mut self,
        label: impl Into<Arc<str>>,
        source: LayoutAnchorPx,
        dest_xy: (f32, f32),
        fly_to_score: bool,
    ) {
        let scale = 92.0;
        let mut rng = rand::rng();
        let yaw = (rng.random::<f32>() - 0.5) * YAW_JITTER;
        self.popups.push(ScorePopup {
            label: label.into(),
            born_at: Instant::now(),
            source_pos: object_popup_source_triple(source, scale),
            dest_xy,
            dest_lift: TABLE_POPUP_LIFT_Z,
            base_scale: scale,
            color: STRUCTURE_CALLOUT_COLOR,
            yaw,
            motion: if fly_to_score {
                PopupMotion::FlyToReel
            } else {
                PopupMotion::Shake
            },
            material: GlyphMaterial::Polychrome,
            timing: PopupMotionTiming::shipping_default(),
            boost_polychrome_halo: true,
        });
    }

    /// Spawn a score popup that grows from zero, loiters at the source, then
    /// flies into the score roller.
    pub fn spawn(
        &mut self,
        label: impl Into<Arc<str>>,
        source: LayoutAnchorPx,
        dest_xy: (f32, f32),
        dest_lift: Option<f32>,
        kind: StepKind,
        magnitude: f32,
        timing: PopupMotionTiming,
    ) {
        let (color, material) = match kind {
            StepKind::Fu => ([0.0, 160.0 / 255.0, 1.0, 1.0], GlyphMaterial::Polychrome),
            StepKind::Han => (HAN_COLOR, GlyphMaterial::Polychrome),
            StepKind::Yen => (GOLD_COLOR, GlyphMaterial::Polychrome),
            StepKind::Final => (FINAL_COLOR, GlyphMaterial::Polychrome),
        };
        let mag = magnitude.abs().max(1.0);
        let scale = 99.0 * (1.0 + (mag.log2() / 12.0).clamp(0.0, 0.48));
        let mut rng = rand::rng();
        let yaw = (rng.random::<f32>() - 0.5) * YAW_JITTER;
        self.popups.push(ScorePopup {
            label: label.into(),
            born_at: Instant::now(),
            source_pos: object_popup_source_triple(source, scale),
            dest_xy,
            dest_lift: dest_lift.unwrap_or(LIFT_BASE),
            base_scale: scale,
            color,
            yaw,
            motion: PopupMotion::FlyToReel,
            material,
            timing,
            boost_polychrome_halo: false,
        });
    }

    /// Spawn a red warning X that shakes in place over a debuffed scorer.
    pub fn spawn_debuff_x(&mut self, source: LayoutAnchorPx, magnitude: f32) {
        let mag = magnitude.abs().max(1.0);
        let scale = 90.0 * (1.0 + (mag.log2() / 10.0).clamp(0.0, 0.35));
        let mut rng = rand::rng();
        let yaw = (rng.random::<f32>() - 0.5) * (YAW_JITTER * 0.6);
        let source_pos = object_popup_source_triple(source, scale);
        self.popups.push(ScorePopup {
            label: Arc::from("X"),
            born_at: Instant::now(),
            source_pos,
            dest_xy: (source_pos[0], source_pos[1]),
            dest_lift: source_pos[2],
            base_scale: scale,
            color: [0.96, 0.24, 0.20, 1.0],
            yaw,
            motion: PopupMotion::Shake,
            material: GlyphMaterial::Polychrome,
            timing: PopupMotionTiming::shipping_default(),
            boost_polychrome_halo: false,
        });
    }

    pub fn update(&mut self, now: Instant) {
        self.popups.retain(|p| {
            let age = now.saturating_duration_since(p.born_at).as_secs_f32();
            age < p.timing.lifetime_secs()
        });
    }

    pub fn clear(&mut self) {
        self.popups.clear();
    }

    pub fn is_active(&self) -> bool {
        !self.popups.is_empty()
    }

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
                    ..Default::default()
                }
            })
            .collect()
    }

    pub fn placements(&self, now: Instant, screen_scale: f32) -> Vec<Object3d> {
        self.popups
            .iter()
            .map(|p| {
                let (px, py, lift_z, scale_mul, alpha, mut emissive) = popup_frame_sample(p, now);
                if p.boost_polychrome_halo {
                    emissive = emissive.max(1.12);
                }

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
                        label: Arc::clone(&p.label),
                        emissive,
                        material: p.material,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                }
            })
            .collect()
    }
}

/// Packed spawn triple with clearance above object-surface anchors.
fn object_popup_source_triple(source: LayoutAnchorPx, glyph_base_scale: f32) -> [f32; 3] {
    let mut triple = source.to_draw_cmd_triple();
    // Modifier-strip / screen-layout sources already float at table-hover height.
    if (source.lift_z - LIFT_BASE).abs() < 1.0 {
        return triple;
    }
    triple[2] += glyph_base_scale * OBJECT_POPUP_CLEARANCE_SCALE + OBJECT_POPUP_CLEARANCE_BASE;
    triple
}

fn popup_frame_sample(p: &ScorePopup, now: Instant) -> (f32, f32, f32, f32, f32, f32) {
    let age = now.saturating_duration_since(p.born_at).as_secs_f32();
    match p.motion {
        PopupMotion::FlyToReel => fly_to_reel_sample(p, age),
        PopupMotion::Shake => shake_sample(p, age),
    }
}

fn fly_to_reel_sample(p: &ScorePopup, age: f32) -> (f32, f32, f32, f32, f32, f32) {
    let t = p.timing;
    let pop_end = t.pop_secs;
    let loiter_end = pop_end + t.loiter_secs;
    let fly_end = loiter_end + t.fly_secs;

    if age < pop_end {
        let local = (age / pop_end.max(1e-6)).clamp(0.0, 1.0);
        let peak = 1.0 + t.overshoot;
        let scale = if local < 0.72 {
            let u = local / 0.72;
            peak * (u * std::f32::consts::FRAC_PI_2).sin()
        } else {
            let u = (local - 0.72) / 0.28;
            peak - (peak - 1.0) * u
        };
        return (
            p.source_pos[0],
            p.source_pos[1],
            p.source_pos[2],
            scale,
            1.0,
            1.05,
        );
    }

    if age < loiter_end {
        return (
            p.source_pos[0],
            p.source_pos[1],
            p.source_pos[2],
            1.0,
            1.0,
            0.92,
        );
    }

    if age < fly_end {
        let local = ((age - loiter_end) / t.fly_secs.max(1e-6)).clamp(0.0, 1.0);
        let eased = if local < 0.5 {
            4.0 * local * local * local
        } else {
            1.0 - (-2.0 * local + 2.0).powi(3) * 0.5
        };
        let dx = p.dest_xy.0 - p.source_pos[0];
        let dy = p.dest_xy.1 - p.source_pos[1];
        let ctrl_x = (p.source_pos[0] + p.dest_xy.0) * 0.5 + dx * 0.11;
        let ctrl_y = (p.source_pos[1] + p.dest_xy.1) * 0.5 + dy * 0.14;
        let one_m = 1.0 - eased;
        let px = one_m * one_m * p.source_pos[0]
            + 2.0 * one_m * eased * ctrl_x
            + eased * eased * p.dest_xy.0;
        let py = one_m * one_m * p.source_pos[1]
            + 2.0 * one_m * eased * ctrl_y
            + eased * eased * p.dest_xy.1;
        let start_lift = p.source_pos[2];
        let lerp_lift = start_lift + (p.dest_lift - start_lift) * eased;
        let arc_env = 4.0 * one_m * eased;
        let lift_z = lerp_lift + LIFT_ARC_PEAK * arc_env;
        let shrink = if local < 0.68 {
            1.0
        } else {
            ((1.0 - local) / 0.32).max(0.0)
        };
        let stream_glow = 4.0 * one_m * eased;
        let emissive = 0.48 + 0.52 * stream_glow;
        return (px, py, lift_z, shrink, 1.0, emissive);
    }

    (p.dest_xy.0, p.dest_xy.1, p.dest_lift, 0.0, 0.0, 0.0)
}

fn shake_sample(p: &ScorePopup, age: f32) -> (f32, f32, f32, f32, f32, f32) {
    let lifetime = 1.18;
    let t = (age / lifetime).clamp(0.0, 1.0);
    let env = if t < 0.8 { 1.0 - t / 0.8 } else { 0.0 };
    let shake = env * p.base_scale * 0.05;
    let px = p.source_pos[0] + (t * 55.0).sin() * shake;
    let py = p.source_pos[1] + (t * 39.0).cos() * shake * 0.55;
    let pop_end = 0.08 * lifetime;
    let (scale_mul, alpha) = if t * lifetime < pop_end {
        let local = (t * lifetime) / pop_end;
        ((local * std::f32::consts::FRAC_PI_2).sin() * 1.10, 1.0)
    } else if t < 0.85 {
        (1.0, 1.0)
    } else {
        let local = (t - 0.85) / 0.15;
        ((1.0 - local).max(0.0), (1.0 - local).max(0.0))
    };
    (px, py, p.source_pos[2], scale_mul, alpha, 1.0)
}

impl Default for ScorePopupSystem {
    fn default() -> Self {
        Self::new()
    }
}
