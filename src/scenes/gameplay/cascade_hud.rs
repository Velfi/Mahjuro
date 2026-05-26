use std::sync::Arc;

use crate::render::draw_cmd::{GlyphMaterial, Object3d, Object3dKind};

#[derive(Clone)]
pub(super) struct CascadeShowcase {
    pub(super) tiles: Vec<crate::core::tile::Tile>,
    pub(super) sets: Vec<crate::core::hand::DetectedMeld>,
}

/// Per-frame cascade HUD state snapshot: the chips/×/mult trio under the
/// plaque and, during the hand-off tween, the merged total flying up into
/// the score reel.
#[derive(Clone, Copy, Debug)]
pub(super) struct CascadeHudState {
    /// Current chip pile reading.
    pub(super) chips: i32,
    /// Current mult reading.
    pub(super) mult: f64,
    /// Product of chips × mult at the end of the cascade. Held fixed from
    /// cascade start so the merge/fly label animates from a stable target.
    pub(super) total: u64,
    /// `Some(eased t)` during the in-place merge sub-phase, else `None`.
    pub(super) merge_t: Option<f32>,
    /// `Some(eased t)` during the flight sub-phase, else `None`.
    pub(super) flight_t: Option<f32>,
}

/// Edge tracker for the hand-off tween — lets us fire one-shot sounds on
/// merge-start, launch (flight-start), and land (flight-end / reel-settled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CascadeHandoffStage {
    /// Cascade not yet in the hand-off window (or no cascade active).
    Pre,
    /// Merge sub-phase in progress — merge sound has fired.
    Merging,
    /// Flight sub-phase in progress — launch sound has fired.
    Flying,
    /// Flight complete — land sound has fired.
    Landed,
}

/// Screen-space geometry of the chips/mult accumulator tokens inside the
/// modifier strip. Produced by `GameplayScene::cascade_token_layout` so the
/// draw path and the popup streaming destinations can't drift apart.
#[derive(Clone, Copy, Debug)]
pub(super) struct CascadeTokenLayout {
    pub(super) chips_center: (f32, f32),
    pub(super) mult_center: (f32, f32),
}

/// Build the extruded-glyph placements for the cascade HUD — the
/// chips/×/mult trio that lives under the plaque and, during hand-off,
/// merges into `= TOTAL` and physically flies up into the score reel.
///
/// Coordinate inputs:
/// - `pad_*`: anchor under the plaque where the trio lives during the
///   main cascade (base steps + per-step reveals + show-total hold).
/// - `reel_*`: destination anchor that the merged label flies toward.
/// - `glyph_scale`: base world-units scale for each glyph (pre-motion).
/// - `plaque_w`: plaque width, used to space the trio laterally.
pub(super) fn build_cascade_hud_placements(
    hud: &CascadeHudState,
    pad_anchor: crate::render::world_space::LayoutAnchorPx,
    reel_anchor: crate::render::world_space::LayoutAnchorPx,
    glyph_scale: f32,
    plaque_w: f32,
) -> Vec<Object3d> {
    let crate::render::world_space::LayoutAnchorPx {
        px: pad_px,
        py: pad_py,
        lift_z: pad_lift,
    } = pad_anchor;
    let crate::render::world_space::LayoutAnchorPx {
        px: reel_px,
        py: reel_py,
        lift_z: reel_lift,
    } = reel_anchor;
    // Color palette mirrors score_popups.rs so the two systems read as
    // siblings: sky blue for Chips (Polychrome), red for Mult (Polychrome),
    // cream for the Metal total / ×.
    const CHIPS_COLOR: [f32; 4] = crate::render::theme::color::LAPIS;
    const MULT_COLOR: [f32; 4] = crate::render::theme::color::RUBY;
    const FINAL_COLOR: [f32; 4] = crate::render::theme::color::PARCHMENT;

    // Lateral spacing for the three labels (chips × mult). Sized off the
    // plaque so the trio lives comfortably beneath it without spilling.
    let spread = plaque_w * 0.28;
    let chips_x = pad_px - spread;
    let times_x = pad_px;
    let mult_x = pad_px + spread;

    // Each frame these three strings are pushed into ExtrudedGlyph labels;
    // the renderer caches the per-string mesh keyed by the &str content, so
    // identical labels across frames hit the cache. Building Arc<str> here
    // means `make_extruded_glyph` doesn't need to re-allocate, and the
    // single Arc lives in the Object3d for one frame.
    let chips_label: Arc<str> = Arc::from(format!("{}", hud.chips));
    let mult_label: Arc<str> = Arc::from(format!("{:.1}x", hud.mult));
    let total_label: Arc<str> = Arc::from(format!("= {}", hud.total));

    // Merge-sub-phase collapse: the three labels slide toward the pad
    // center as `merge_t` climbs from 0 → 1, while the `= TOTAL` label
    // fades in at the center. At merge_t == 1.0 the trio is fully collapsed
    // and the total is fully visible.
    let merge_t = hud.merge_t.unwrap_or(0.0);
    let trio_alpha = (1.0 - merge_t).max(0.0);
    let total_alpha_merge = merge_t;

    // Flight sub-phase: the (now-merged) total physically travels from pad
    // to reel, scaling down and fading into the reel as it lands.
    let flight_t = hud.flight_t.unwrap_or(0.0);
    // Quadratic bezier arc: control point raised above the midpoint in
    // world-Z so the label arcs up over the plaque rather than sliding
    // flat through it. XY is biased toward the reel so the path reads as
    // feeding the counter.
    let dx = reel_px - pad_px;
    let dy = reel_py - pad_py;
    let ctrl_px = (pad_px + reel_px) * 0.5 + dx * 0.12;
    let ctrl_py = (pad_py + reel_py) * 0.5 + dy * 0.15;
    let ctrl_lift = pad_lift.max(reel_lift) + 288.0;
    let one_m = 1.0 - flight_t;
    let total_px =
        one_m * one_m * pad_px + 2.0 * one_m * flight_t * ctrl_px + flight_t * flight_t * reel_px;
    let total_py =
        one_m * one_m * pad_py + 2.0 * one_m * flight_t * ctrl_py + flight_t * flight_t * reel_py;
    let total_lift = one_m * one_m * pad_lift
        + 2.0 * one_m * flight_t * ctrl_lift
        + flight_t * flight_t * reel_lift;
    // Shrink into the reel as it lands. Hold full size until ~58% of the
    // flight, then taper so the label visibly "absorbs" into the reel.
    let flight_scale_mul = if flight_t < 0.58 {
        1.0
    } else {
        let k = (flight_t - 0.58) / 0.42;
        1.0 - k * 0.94
    };
    // Final absorption fade near landing.
    let total_alpha_flight = if flight_t < 0.85 {
        1.0
    } else {
        1.0 - (flight_t - 0.85) / 0.15
    };

    let mut out: Vec<Object3d> = Vec::new();

    // Trio: CHIPS × MULT under the plaque. Fade out during merge.
    if trio_alpha > 0.001 {
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        // Collapse toward the center as merge_t → 1.
        let cx = lerp(chips_x, times_x, merge_t);
        let mx = lerp(mult_x, times_x, merge_t);
        let scale_mul = 1.0 - merge_t * 0.42;

        out.push(make_extruded_glyph(
            chips_label,
            [cx, pad_py, pad_lift],
            glyph_scale * scale_mul,
            with_alpha(CHIPS_COLOR, trio_alpha),
            GlyphMaterial::Polychrome,
            0.9 + merge_t * 0.12,
        ));
        out.push(make_extruded_glyph(
            Arc::from("x"),
            [times_x, pad_py, pad_lift],
            glyph_scale * 0.85,
            with_alpha(FINAL_COLOR, trio_alpha),
            GlyphMaterial::Metal,
            0.82 + merge_t * 0.1,
        ));
        out.push(make_extruded_glyph(
            mult_label,
            [mx, pad_py, pad_lift],
            glyph_scale * scale_mul,
            with_alpha(MULT_COLOR, trio_alpha),
            GlyphMaterial::Polychrome,
            0.9 + merge_t * 0.12,
        ));
    }

    // Merged total: fades in at the pad during merge, then flies to the
    // reel during the flight sub-phase.
    let total_alpha = if hud.flight_t.is_some() {
        total_alpha_flight
    } else {
        total_alpha_merge
    };
    let total_emissive = if hud.flight_t.is_some() {
        0.68 + 0.42 * (1.0 - flight_t).powf(1.15)
    } else {
        0.85 + 0.22 * merge_t
    };
    if total_alpha > 0.001 {
        out.push(make_extruded_glyph(
            total_label,
            [total_px, total_py, total_lift],
            glyph_scale * flight_scale_mul * 1.1,
            with_alpha(FINAL_COLOR, total_alpha),
            GlyphMaterial::Metal,
            total_emissive.clamp(0.35, 1.35),
        ));
    }

    out
}

fn make_extruded_glyph(
    label: Arc<str>,
    pos: [f32; 3],
    scale: f32,
    color: [f32; 4],
    material: GlyphMaterial,
    emissive: f32,
) -> Object3d {
    Object3d {
        pos,
        extents: [1.0, 1.0, 1.0],
        rotation: [0.0, 0.0, 0.0],
        color,
        kind: Object3dKind::ExtrudedGlyph {
            scale,
            rotation_x: 0.08,
            rotation_y: 0.0,
            label,
            emissive,
            material,
        },
        hover_target: 0.0,
        anim_id: 0,
    }
}

#[inline]
fn with_alpha(mut c: [f32; 4], a: f32) -> [f32; 4] {
    c[3] *= a.clamp(0.0, 1.0);
    c
}
