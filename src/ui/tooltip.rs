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

/// Rim thickness in pixels (brass quad extends this far outside the inner fill rect).
pub const FRAME_BORDER_PX: f32 = 2.0;

/// Panel fill opacity for standard tooltip frames (`WALNUT_DEEP` — dark walnut).
pub const FILL_ALPHA: f32 = 0.92;

/// Brass rim opacity for standard tooltip panels.
pub const RIM_ALPHA: f32 = 0.45;

/// Push brass rim + dark walnut fill. `(left, top)` is the **inner** fill origin; the rim draws outside it.
pub fn push_tooltip_frame_quads(
    out: &mut Vec<GpuInstance>,
    left: f32,
    top: f32,
    inner_w: f32,
    inner_h: f32,
) {
    let b = FRAME_BORDER_PX;
    out.push(GpuInstance {
        rect: [left - b, top - b, inner_w + b * 2.0, inner_h + b * 2.0],
        color: [color::BRASS[0], color::BRASS[1], color::BRASS[2], RIM_ALPHA],
    });
    out.push(GpuInstance {
        rect: [left, top, inner_w, inner_h],
        color: [
            color::WALNUT_DEEP[0],
            color::WALNUT_DEEP[1],
            color::WALNUT_DEEP[2],
            FILL_ALPHA,
        ],
    });
}
