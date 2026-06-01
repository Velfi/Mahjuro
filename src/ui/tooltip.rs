//! ## Tooltip / transient inspect panels (2D)
//!
//! **Frame** — floating overlays that explain a focused or hovered target share one look:
//!
//! | Layer | Role |
//! |-------|------|
//! | Outer rim | [`crate::render::theme::color::BRASS`] at [`RIM_ALPHA`] — thin brass frame |
//! | Fill | [`crate::render::theme::color::WALNUT_DEEP`] at [`FILL_ALPHA`] — dark walnut panel |
//!
//! **Content** — multi-line inspect uses [`crate::ui::inspect_plaque::push_focus_tooltip_panel_2d`]:
//! title [`color::CHAMPAGNE`] (HEADING tier), optional accent line (price/tier — caller color),
//! body [`color::PARCHMENT`] (BODY tier). Transient overlays set [`crate::render::wgpu_renderer::TextLabel::no_glossary`].
//!
//! **Compact hovers** — single-line `ButtonDef::hover_label` tooltips (`main/draw.rs`) use
//! [`push_tooltip_frame_quads`] with the same rim/fill and proportional body-sized text.

use crate::render::theme::color;
use crate::render::wgpu_renderer::GpuInstance;

/// Reference rim at ~720p; prefer [`crate::render::theme::metrics::tooltip_border_px`].
pub const FRAME_BORDER_PX: f32 = 2.0;

/// Panel fill opacity for standard tooltip frames (`WALNUT_DEEP` — dark walnut).
pub const FILL_ALPHA: f32 = 0.92;

/// Brass rim opacity for standard tooltip panels.
pub const RIM_ALPHA: f32 = 0.58;

/// Push brass rim + dark walnut fill. `(left, top)` is the **inner** fill origin; the rim draws outside it.
pub fn push_tooltip_frame_quads(
    out: &mut Vec<GpuInstance>,
    left: f32,
    top: f32,
    inner_w: f32,
    inner_h: f32,
    border_px: f32,
) {
    let b = border_px.max(1.0);
    out.push(GpuInstance {
        rect: [left - b, top - b, inner_w + b * 2.0, inner_h + b * 2.0],
        color: [color::BRASS[0], color::BRASS[1], color::BRASS[2], RIM_ALPHA],
        user: 0,
    });
    out.push(GpuInstance {
        rect: [left, top, inner_w, inner_h],
        color: [
            color::WALNUT_DEEP[0],
            color::WALNUT_DEEP[1],
            color::WALNUT_DEEP[2],
            FILL_ALPHA,
        ],
        user: 0,
    });
}

/// Scanline-filled convex polygon (speech-bubble tail).
fn push_polygon_fill_quads(out: &mut Vec<GpuInstance>, verts: &[[f32; 2]], color: [f32; 4]) {
    if verts.len() < 3 {
        return;
    }
    let y_min = verts
        .iter()
        .map(|v| v[1])
        .fold(f32::INFINITY, f32::min)
        .floor();
    let y_max = verts
        .iter()
        .map(|v| v[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil();
    let n = verts.len();
    let mut y = y_min;
    while y <= y_max {
        let yc = y + 0.5;
        let mut xs = Vec::new();
        for i in 0..n {
            let a = verts[i];
            let b = verts[(i + 1) % n];
            let dy = b[1] - a[1];
            if dy.abs() <= 1e-5 {
                continue;
            }
            if (a[1] <= yc && b[1] > yc) || (b[1] <= yc && a[1] > yc) {
                let t = (yc - a[1]) / dy;
                xs.push(a[0] + t * (b[0] - a[0]));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.chunks(2) {
            if pair.len() == 2 {
                let w = pair[1] - pair[0];
                if w > 0.05 {
                    out.push(GpuInstance {
                        rect: [pair[0], y, w, 1.0],
                        color,
                        user: 0,
                    });
                }
            }
        }
        y += 1.0;
    }
}

#[inline]
fn quad_bezier2(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    [
        u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0],
        u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1],
    ]
}

#[inline]
fn quad_bezier2_tangent(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    [
        2.0 * u * (p1[0] - p0[0]) + 2.0 * t * (p2[0] - p1[0]),
        2.0 * u * (p1[1] - p0[1]) + 2.0 * t * (p2[1] - p1[1]),
    ]
}

/// Tapered tail along a quadratic bezier from `base` → `tip`, bulging through `control`.
fn push_curved_tail_quads(
    out: &mut Vec<GpuInstance>,
    base: [f32; 2],
    control: [f32; 2],
    tip: [f32; 2],
    half_width: f32,
    color: [f32; 4],
) {
    const STEPS: usize = 22;
    let mut poly = Vec::with_capacity(STEPS * 2 + 2);
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let p = quad_bezier2(base, control, tip, t);
        let tan = quad_bezier2_tangent(base, control, tip, t);
        let len = (tan[0] * tan[0] + tan[1] * tan[1]).sqrt().max(1e-5);
        let nx = -tan[1] / len;
        let ny = tan[0] / len;
        let w = half_width * (1.0 - t);
        poly.push([p[0] + nx * w, p[1] + ny * w]);
    }
    for i in (0..=STEPS).rev() {
        let t = i as f32 / STEPS as f32;
        let p = quad_bezier2(base, control, tip, t);
        let tan = quad_bezier2_tangent(base, control, tip, t);
        let len = (tan[0] * tan[0] + tan[1] * tan[1]).sqrt().max(1e-5);
        let nx = -tan[1] / len;
        let ny = tan[0] / len;
        let w = half_width * (1.0 - t);
        poly.push([p[0] - nx * w, p[1] - ny * w]);
    }
    push_polygon_fill_quads(out, &poly, color);
}

/// Walnut + brass speech bubble with a curved tail from the panel lower-left.
///
/// Tail geometry goes in `quads`; rounded panel body goes in `squircles` (post-tonemap squircle pass).
pub fn push_speech_bubble_overlay(
    quads: &mut Vec<GpuInstance>,
    squircles: &mut Vec<GpuInstance>,
    panel_x: f32,
    panel_y: f32,
    panel_w: f32,
    panel_h: f32,
    border_px: f32,
    tail_tip: [f32; 2],
    tail_base_half_w: f32,
) {
    let b = border_px.max(1.0);
    let fill = [
        color::WALNUT_DEEP[0],
        color::WALNUT_DEEP[1],
        color::WALNUT_DEEP[2],
        FILL_ALPHA,
    ];
    let rim = [color::BRASS[0], color::BRASS[1], color::BRASS[2], RIM_ALPHA];

    // Lower-left of the panel, inset so the squircle corner covers the join.
    let tail_base = [
        panel_x + panel_w * 0.14,
        panel_y + panel_h * 0.90,
    ];
    // Bulge the curve downward/outward toward the tip (moon).
    let control = [
        tail_base[0] - (tail_base[0] - tail_tip[0]).abs() * 0.22,
        (tail_base[1] + tail_tip[1]) * 0.5 + tail_base_half_w * 1.6,
    ];

    push_curved_tail_quads(
        quads,
        tail_base,
        control,
        tail_tip,
        tail_base_half_w + b * 0.45,
        rim,
    );
    push_curved_tail_quads(quads, tail_base, control, tail_tip, tail_base_half_w, fill);

    squircles.push(GpuInstance {
        rect: [
            panel_x - b,
            panel_y - b,
            panel_w + b * 2.0,
            panel_h + b * 2.0,
        ],
        color: rim,
        user: 0,
    });
    squircles.push(GpuInstance {
        rect: [panel_x, panel_y, panel_w, panel_h],
        color: fill,
        user: 0,
    });
}
