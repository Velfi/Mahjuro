use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use crate::render::draw_cmd::{CascadeTokenKind, GlyphMaterial, Object3d, Object3dKind};

pub(super) fn structure_preview_chip_stack_count(final_chips: i32) -> usize {
    if final_chips <= 0 {
        0
    } else {
        ((final_chips + 74) / 75).clamp(1, 12) as usize
    }
}

pub(super) fn structure_preview_mult_stack_count(final_mult: f64) -> usize {
    if final_mult <= 0.0 {
        0
    } else {
        final_mult.ceil().clamp(1.0, 10.0) as usize
    }
}

/// Placement for a pile of preview tokens: pixel-space center anchor,
/// base lift above the felt, and the per-token bounding box extents.
pub(super) struct PreviewPilePlacement {
    pub(super) center_x: f32,
    pub(super) center_y: f32,
    pub(super) base_lift: f32,
    pub(super) extents: [f32; 3],
}

pub(super) fn push_structure_preview_pile(
    out: &mut Vec<Object3d>,
    kind: CascadeTokenKind,
    count: usize,
    placement: PreviewPilePlacement,
    pulse: f32,
    seed: u64,
) {
    let PreviewPilePlacement {
        center_x,
        center_y,
        base_lift,
        extents,
    } = placement;
    if count == 0 {
        return;
    }

    let mut rng = StdRng::seed_from_u64(seed);
    let overlap_x = extents[0] * 0.82;
    let overlap_z = extents[2] * 0.82;
    let overlap_x2 = overlap_x * overlap_x;
    let overlap_z2 = overlap_z * overlap_z;
    let max_radius_x = extents[0] * (0.70 + (count as f32).sqrt() * 0.24);
    let max_radius_z = extents[2] * (0.52 + (count as f32).sqrt() * 0.18);
    let lift_step = (extents[1] * 0.82).max(2.0);
    const CANDIDATES_PER_TOKEN: u32 = 14;

    let mut placed: Vec<(f32, f32, f32)> = Vec::with_capacity(count);
    for _ in 0..count {
        let mut best: Option<(f32, f32, f32, f32)> = None;
        for _ in 0..CANDIDATES_PER_TOKEN {
            let angle = rng.random_range(0.0..std::f32::consts::TAU);
            let radius_bias = rng.random::<f32>().powf(1.65);
            let lx = angle.cos() * max_radius_x * radius_bias;
            let lz = angle.sin() * max_radius_z * radius_bias;
            let radial_norm = ((lx / max_radius_x).powi(2) + (lz / max_radius_z).powi(2)).sqrt();

            let mut support_y = base_lift;
            for (ox, oz, top_y) in &placed {
                let dx = lx - ox;
                let dz = lz - oz;
                let overlap = (dx * dx) / overlap_x2 + (dz * dz) / overlap_z2;
                if overlap < 1.0 && *top_y > support_y {
                    support_y = *top_y;
                }
            }

            match best {
                None => best = Some((lx, lz, support_y, radial_norm)),
                Some((_, _, by, _)) if support_y < by - 0.01 => {
                    best = Some((lx, lz, support_y, radial_norm));
                }
                Some((_, _, by, br)) if (support_y - by).abs() <= 0.01 && radial_norm < br => {
                    best = Some((lx, lz, support_y, radial_norm));
                }
                _ => {}
            }
        }

        let (lx, lz, support_y, _) = best.unwrap();
        let world_y = support_y + extents[1] * 0.5;
        placed.push((lx, lz, support_y + lift_step));
        out.push(Object3d {
            pos: [center_x + lx, center_y + lz, world_y],
            extents,
            rotation: glam::Mat4::IDENTITY,
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::CascadeToken { kind, pulse },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });
    }
}

#[derive(Clone)]
pub(super) struct CascadeShowcase {
    pub(super) tiles: Vec<crate::core::tile::Tile>,
    pub(super) sets: Vec<crate::core::hand::DetectedSet>,
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
    pub(super) pill_w: f32,
    pub(super) pill_h: f32,
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
    const CHIPS_COLOR: [f32; 4] = [0.55, 0.78, 1.00, 1.0];
    const MULT_COLOR: [f32; 4] = [1.00, 0.42, 0.38, 1.0];
    const FINAL_COLOR: [f32; 4] = [1.00, 0.95, 0.76, 1.0];

    // Lateral spacing for the three labels (chips × mult). Sized off the
    // plaque so the trio lives comfortably beneath it without spilling.
    let spread = plaque_w * 0.28;
    let chips_x = pad_px - spread;
    let times_x = pad_px;
    let mult_x = pad_px + spread;

    let chips_label = format!("{}", hud.chips);
    let mult_label = format!("{:.1}x", hud.mult);
    let total_label = format!("= {}", hud.total);

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
    // flat through it.
    let ctrl_px = (pad_px + reel_px) * 0.5;
    let ctrl_py = (pad_py + reel_py) * 0.5;
    let ctrl_lift = pad_lift.max(reel_lift) + 180.0;
    let one_m = 1.0 - flight_t;
    let total_px =
        one_m * one_m * pad_px + 2.0 * one_m * flight_t * ctrl_px + flight_t * flight_t * reel_px;
    let total_py =
        one_m * one_m * pad_py + 2.0 * one_m * flight_t * ctrl_py + flight_t * flight_t * reel_py;
    let total_lift = one_m * one_m * pad_lift
        + 2.0 * one_m * flight_t * ctrl_lift
        + flight_t * flight_t * reel_lift;
    // Shrink into the reel as it lands. Hold full size until ~60% of the
    // flight, then taper so the label visibly "absorbs" into the reel.
    let flight_scale_mul = if flight_t < 0.6 {
        1.0
    } else {
        let k = (flight_t - 0.6) / 0.4;
        1.0 - k * 0.9
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
        let scale_mul = 1.0 - merge_t * 0.35;

        out.push(make_extruded_glyph(
            chips_label,
            [cx, pad_py, pad_lift],
            glyph_scale * scale_mul,
            with_alpha(CHIPS_COLOR, trio_alpha),
            GlyphMaterial::Polychrome,
        ));
        out.push(make_extruded_glyph(
            "x".to_string(),
            [times_x, pad_py, pad_lift],
            glyph_scale * 0.85,
            with_alpha(FINAL_COLOR, trio_alpha),
            GlyphMaterial::Metal,
        ));
        out.push(make_extruded_glyph(
            mult_label,
            [mx, pad_py, pad_lift],
            glyph_scale * scale_mul,
            with_alpha(MULT_COLOR, trio_alpha),
            GlyphMaterial::Polychrome,
        ));
    }

    // Merged total: fades in at the pad during merge, then flies to the
    // reel during the flight sub-phase.
    let total_alpha = if hud.flight_t.is_some() {
        total_alpha_flight
    } else {
        total_alpha_merge
    };
    if total_alpha > 0.001 {
        out.push(make_extruded_glyph(
            total_label,
            [total_px, total_py, total_lift],
            glyph_scale * flight_scale_mul * 1.1,
            with_alpha(FINAL_COLOR, total_alpha),
            GlyphMaterial::Metal,
        ));
    }

    out
}

fn make_extruded_glyph(
    label: String,
    pos: [f32; 3],
    scale: f32,
    color: [f32; 4],
    material: GlyphMaterial,
) -> Object3d {
    Object3d {
        pos,
        extents: [1.0, 1.0, 1.0],
        rotation: glam::Mat4::IDENTITY,
        color,
        kind: Object3dKind::ExtrudedGlyph {
            scale,
            rotation_x: 0.08,
            rotation_y: 0.0,
            label,
            emissive: 0.9,
            material,
        },
        hover_target: 0.0,
        anim_id: 0,
        arrange_name: None,
    }
}

#[inline]
fn with_alpha(mut c: [f32; 4], a: f32) -> [f32; 4] {
    c[3] *= a.clamp(0.0, 1.0);
    c
}
