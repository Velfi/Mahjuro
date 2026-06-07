//! Floating relic-flavor caption shadow tuning + debug lab helpers.
//!
//! Production defaults live in [`FloatingFlavorShadowTuning::DEFAULT`]; the
//! Text Shadow Lab cycles and nudges these values against sample copy.

use crate::decal::{DecalFonts, measure_flavor_spans_layout};
use crate::theme::color;
use crate::wgpu_renderer::GradientQuadInstance;
use mahjuro_core::core::relic::RelicFlavorSpan;

/// Soft gradient backer behind bottom-anchored relic / staircase flavor copy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingFlavorShadowTuning {
    /// Horizontal inset from window edges as a fraction of window width.
    pub margin_x_frac: f32,
    /// Maximum caption band width in px.
    pub band_max_w: f32,
    /// Distance from the bottom edge before the caption band (fraction of height).
    pub bottom_margin_frac: f32,
    /// Horizontal padding added outside the measured copy width (fraction of copy width).
    pub pad_x_frac: f32,
    /// Extra space above the copy block inside the shadow (× body font px).
    pub pad_top_body: f32,
    /// Extra space below the copy block inside the shadow (× band height).
    pub pad_bottom_band: f32,
    /// Small fudge added to measured copy height (× body font px).
    pub content_pad_body: f32,
    /// Walnut-ink shadow fill alpha.
    pub shadow_alpha: f32,
    /// Horizontal edge fade (see `gradient_quad.wgsl` — lower = darker ends).
    pub feather_x: f32,
    /// Radial mix (0 = axial rect, 1 = radial halo).
    pub feather_y: f32,
    /// Vertical edge fade (0 = use `feather_x` for both axes).
    pub feather_z: f32,
}

impl Default for FloatingFlavorShadowTuning {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl FloatingFlavorShadowTuning {
    pub const DEFAULT: Self = Self {
        margin_x_frac: 0.035,
        band_max_w: 1040.0,
        bottom_margin_frac: 0.055,
        pad_x_frac: 0.10,
        pad_top_body: 0.38,
        pad_bottom_band: 0.16,
        content_pad_body: 0.20,
        shadow_alpha: 0.86,
        feather_x: 0.16,
        feather_y: 0.08,
        feather_z: 0.40,
    };

    pub fn shadow_color(&self) -> [f32; 4] {
        color::alpha(color::WALNUT_INK, self.shadow_alpha)
    }

    pub fn gradient_feather(&self) -> [f32; 4] {
        [self.feather_x, self.feather_y, self.feather_z, 0.0]
    }
}

/// Screen-space layout for a single floating flavor caption + its gradient shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingFlavorCaptionLayout {
    pub band_left: f32,
    pub band_top: f32,
    pub band_w: f32,
    pub band_h: f32,
    /// Bottom-aligned copy block (matches raster vertical extent).
    pub content_top: f32,
    pub content_h: f32,
    pub content_left: f32,
    pub content_w: f32,
    pub shadow_rect: [f32; 4],
}

impl FloatingFlavorCaptionLayout {
    pub fn text_rect(&self) -> [f32; 4] {
        [self.band_left, self.band_top, self.band_w, self.band_h]
    }

    pub fn content_rect(&self) -> [f32; 4] {
        [self.content_left, self.content_top, self.content_w, self.content_h]
    }

    pub fn gradient_quad(&self, tuning: &FloatingFlavorShadowTuning) -> GradientQuadInstance {
        GradientQuadInstance {
            rect: self.shadow_rect,
            color: tuning.shadow_color(),
            feather: tuning.gradient_feather(),
        }
    }
}

/// Compute band + shadow geometry for bottom-anchored flavor copy.
pub fn layout_floating_flavor_caption(
    window_w: f32,
    window_h: f32,
    body_px: f32,
    line_step: f32,
    content_lines: usize,
    extra_bottom_reserve: f32,
    tuning: &FloatingFlavorShadowTuning,
) -> FloatingFlavorCaptionLayout {
    let _ = line_step;
    let margin_x = window_w * tuning.margin_x_frac;
    let band_w = (window_w - 2.0 * margin_x).min(tuning.band_max_w);
    let band_left = (window_w - band_w) * 0.5;

    let content_lines = content_lines.max(1);
    let band_h = flavor_caption_band_height(body_px, line_step, content_lines, window_h);
    let text_block_h = line_step * content_lines as f32;
    let bottom_margin = window_h * tuning.bottom_margin_frac + extra_bottom_reserve.max(0.0);
    let band_top = window_h - bottom_margin - band_h;

    caption_layout_from_band(
        band_left,
        band_top,
        band_w,
        band_h,
        body_px,
        text_block_h,
        None,
        tuning,
    )
}

/// Preferred path: measure with the same font + wrap rules as the text rasterizer.
pub fn layout_floating_flavor_caption_for_spans(
    window_w: f32,
    window_h: f32,
    fonts: &DecalFonts<'_>,
    spans: &[RelicFlavorSpan],
    target_font_px: f32,
    min_font_px: f32,
    extra_bottom_reserve: f32,
    tuning: &FloatingFlavorShadowTuning,
) -> FloatingFlavorCaptionLayout {
    let margin_x = window_w * tuning.margin_x_frac;
    let band_w = (window_w - 2.0 * margin_x).min(tuning.band_max_w);
    let band_left = (window_w - band_w) * 0.5;
    let band_w_u = band_w.max(1.0) as u32;

    let metrics_at_target = measure_flavor_spans_layout(
        fonts,
        spans,
        band_w_u,
        u32::MAX,
        target_font_px,
        min_font_px,
    );
    let ideal_band_h = metrics_at_target.text_block_h + target_font_px * 0.5;
    let band_h = ideal_band_h
        .min(window_h * 0.25)
        .max(target_font_px * 2.0);
    let metrics = if ideal_band_h <= band_h + 0.5 {
        metrics_at_target
    } else {
        measure_flavor_spans_layout(
            fonts,
            spans,
            band_w_u,
            band_h.max(1.0) as u32,
            target_font_px,
            min_font_px,
        )
    };

    let bottom_margin = window_h * tuning.bottom_margin_frac + extra_bottom_reserve.max(0.0);
    let band_top = window_h - bottom_margin - band_h;
    caption_layout_from_band(
        band_left,
        band_top,
        band_w,
        band_h,
        target_font_px,
        metrics.text_block_h,
        Some(metrics.text_block_w),
        tuning,
    )
}

/// Same shadow math as production, but with an explicit band origin (lab stage).
pub fn layout_floating_flavor_caption_at_band_top_for_spans(
    window_w: f32,
    band_top: f32,
    max_band_h: f32,
    fonts: &DecalFonts<'_>,
    spans: &[RelicFlavorSpan],
    target_font_px: f32,
    min_font_px: f32,
    tuning: &FloatingFlavorShadowTuning,
) -> FloatingFlavorCaptionLayout {
    let margin_x = window_w * tuning.margin_x_frac;
    let band_w = (window_w - 2.0 * margin_x).min(tuning.band_max_w);
    let band_left = (window_w - band_w) * 0.5;
    let band_w_u = band_w.max(1.0) as u32;

    let metrics_at_target = measure_flavor_spans_layout(
        fonts,
        spans,
        band_w_u,
        u32::MAX,
        target_font_px,
        min_font_px,
    );
    let ideal_band_h = metrics_at_target.text_block_h + target_font_px * 0.5;
    let band_h = ideal_band_h
        .min(max_band_h * 0.45)
        .max(target_font_px * 2.0)
        .min(max_band_h);
    let metrics = if ideal_band_h <= band_h + 0.5 {
        metrics_at_target
    } else {
        measure_flavor_spans_layout(
            fonts,
            spans,
            band_w_u,
            band_h.max(1.0) as u32,
            target_font_px,
            min_font_px,
        )
    };

    caption_layout_from_band(
        band_left,
        band_top + max_band_h - band_h,
        band_w,
        band_h,
        target_font_px,
        metrics.text_block_h,
        Some(metrics.text_block_w),
        tuning,
    )
}

/// Legacy lab helper — prefer [`layout_floating_flavor_caption_at_band_top_for_spans`].
pub fn layout_floating_flavor_caption_at_band_top(
    window_w: f32,
    band_top: f32,
    max_band_h: f32,
    body_px: f32,
    line_step: f32,
    content_lines: usize,
    tuning: &FloatingFlavorShadowTuning,
) -> FloatingFlavorCaptionLayout {
    let content_lines = content_lines.max(1);
    let margin_x = window_w * tuning.margin_x_frac;
    let band_w = (window_w - 2.0 * margin_x).min(tuning.band_max_w);
    let band_left = (window_w - band_w) * 0.5;
    let band_h = flavor_caption_band_height(body_px, line_step, content_lines, max_band_h);
    caption_layout_from_band(
        band_left,
        band_top,
        band_w,
        band_h,
        body_px,
        line_step * content_lines as f32,
        None,
        tuning,
    )
}

fn flavor_caption_band_height(
    body_px: f32,
    line_step: f32,
    content_lines: usize,
    height_budget: f32,
) -> f32 {
    let content_lines = content_lines.max(1);
    (line_step * content_lines as f32 + body_px * 0.5)
        .min(height_budget * 0.25)
        .max(body_px * 2.0)
}

fn caption_layout_from_band(
    band_left: f32,
    band_top: f32,
    band_w: f32,
    band_h: f32,
    body_px: f32,
    text_block_h: f32,
    text_block_w: Option<f32>,
    tuning: &FloatingFlavorShadowTuning,
) -> FloatingFlavorCaptionLayout {
    let content_h = text_block_h + body_px * tuning.content_pad_body;
    let content_w = text_block_w.unwrap_or(band_w).clamp(0.0, band_w);
    let content_left = band_left + (band_w - content_w) * 0.5;
    let pad_x = content_w * tuning.pad_x_frac;
    let pad_top = body_px * tuning.pad_top_body;
    let pad_bottom = band_h * tuning.pad_bottom_band;

    let text_bottom = band_top + band_h;
    let content_top = text_bottom - content_h;
    let shadow_top = content_top - pad_top;
    let shadow_bottom = text_bottom + pad_bottom;
    let shadow_h = (shadow_bottom - shadow_top).max(1.0);
    let shadow_left = content_left - pad_x;
    let shadow_w = content_w + 2.0 * pad_x;

    FloatingFlavorCaptionLayout {
        band_left,
        band_top,
        band_w,
        band_h,
        content_top,
        content_h,
        content_left,
        content_w,
        shadow_rect: [shadow_left, shadow_top, shadow_w, shadow_h],
    }
}

/// Lab-only: which scalar field is currently selected for nudging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningField {
    PadTopBody,
    PadBottomBand,
    PadXFrac,
    ShadowAlpha,
    FeatherX,
    FeatherY,
    FeatherZ,
    BottomMarginFrac,
}

impl TuningField {
    pub const ALL: &[Self] = &[
        Self::PadTopBody,
        Self::PadBottomBand,
        Self::PadXFrac,
        Self::ShadowAlpha,
        Self::FeatherX,
        Self::FeatherY,
        Self::FeatherZ,
        Self::BottomMarginFrac,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::PadTopBody => "pad_top (× body)",
            Self::PadBottomBand => "pad_bottom (× band)",
            Self::PadXFrac => "pad_x (× copy_w)",
            Self::ShadowAlpha => "shadow_alpha",
            Self::FeatherX => "feather_x (horizontal fade)",
            Self::FeatherY => "feather_y (radial mix)",
            Self::FeatherZ => "feather_z (vertical fade)",
            Self::BottomMarginFrac => "bottom_margin (× h)",
        }
    }

    pub fn next(self) -> Self {
        let all = Self::ALL;
        let idx = all.iter().position(|&f| f == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    pub fn prev(self) -> Self {
        let all = Self::ALL;
        let idx = all.iter().position(|&f| f == self).unwrap_or(0);
        all[(idx + all.len() - 1) % all.len()]
    }

    pub fn nudge(self, tuning: &mut FloatingFlavorShadowTuning, delta: f32) {
        match self {
            Self::PadTopBody => tuning.pad_top_body = (tuning.pad_top_body + delta).clamp(0.0, 1.2),
            Self::PadBottomBand => {
                tuning.pad_bottom_band = (tuning.pad_bottom_band + delta).clamp(0.0, 0.45)
            }
            Self::PadXFrac => tuning.pad_x_frac = (tuning.pad_x_frac + delta).clamp(0.0, 0.25),
            Self::ShadowAlpha => tuning.shadow_alpha = (tuning.shadow_alpha + delta).clamp(0.0, 1.0),
            Self::FeatherX => tuning.feather_x = (tuning.feather_x + delta).clamp(0.02, 0.95),
            Self::FeatherY => tuning.feather_y = (tuning.feather_y + delta).clamp(0.0, 1.0),
            Self::FeatherZ => tuning.feather_z = (tuning.feather_z + delta).clamp(0.02, 0.95),
            Self::BottomMarginFrac => {
                tuning.bottom_margin_frac =
                    (tuning.bottom_margin_frac + delta).clamp(0.02, 0.14)
            }
        }
    }

    pub fn value(self, tuning: &FloatingFlavorShadowTuning) -> f32 {
        match self {
            Self::PadTopBody => tuning.pad_top_body,
            Self::PadBottomBand => tuning.pad_bottom_band,
            Self::PadXFrac => tuning.pad_x_frac,
            Self::ShadowAlpha => tuning.shadow_alpha,
            Self::FeatherX => tuning.feather_x,
            Self::FeatherY => tuning.feather_y,
            Self::FeatherZ => tuning.feather_z,
            Self::BottomMarginFrac => tuning.bottom_margin_frac,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_hugs_bottom_aligned_copy() {
        let tuning = FloatingFlavorShadowTuning::DEFAULT;
        let body_px = 32.0;
        let line_step = body_px * 1.22;
        let layout = layout_floating_flavor_caption(
            1920.0,
            1080.0,
            body_px,
            line_step,
            1,
            0.0,
            &tuning,
        );
        let text_bottom = layout.band_top + layout.band_h;
        assert!(
            layout.content_top + layout.content_h <= text_bottom + 0.5,
            "copy block should sit on the band bottom"
        );
        assert!(
            layout.shadow_rect[1] <= layout.content_top + 0.5,
            "shadow should start at or above copy top"
        );
        let shadow_bottom = layout.shadow_rect[1] + layout.shadow_rect[3];
        assert!(
            shadow_bottom >= text_bottom,
            "shadow should extend through copy baseline"
        );
    }

    #[test]
    fn shadow_height_tracks_copy_not_empty_band() {
        let tuning = FloatingFlavorShadowTuning::DEFAULT;
        let body_px = 32.0;
        let line_step = body_px * 1.22;
        let one_line = layout_floating_flavor_caption(
            1920.0,
            1080.0,
            body_px,
            line_step,
            1,
            0.0,
            &tuning,
        );
        let three_lines = layout_floating_flavor_caption(
            1920.0,
            1080.0,
            body_px,
            line_step,
            3,
            0.0,
            &tuning,
        );
        assert!(
            three_lines.shadow_rect[3] > one_line.shadow_rect[3],
            "taller copy should produce a taller shadow"
        );
    }
}
