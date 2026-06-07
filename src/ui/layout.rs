//! Window scale anchor for gameplay HUD and cross-scene physical sizing.
//!
//! Prop placement on the gameplay table comes from [`gameplay.glb`](../../assets/3d/gameplay.glb)
//! marker empties ([`crate::scenes::gameplay::glb_anchors`]). This module only derives the
//! reference hand-tile width used by [`LayoutResult::mm`] and metadata for GLB projection.

use crate::game::game_mode::HAND_SIZE;

/// Window dimensions for widget layout helpers.
#[derive(Clone, Copy, Debug)]
pub struct ViewportCtx {
    pub window_w: f32,
    pub window_h: f32,
}

/// Per-frame window scale anchor.
#[derive(Clone, Debug)]
pub struct LayoutResult {
    pub window_w: f32,
    pub window_h: f32,
    /// Width of one reference hand slot (14-tile rack sizing anchor).
    pub hand_slot_w: f32,
    /// Height of the hand strip (fed into GLB marker projection).
    pub hand_slot_h: f32,
    /// Reference hand slot count ([`HAND_SIZE`], typically 14).
    pub hand_slot_count: usize,
}

/// Real-world width of a Japanese mahjong tile, in millimeters. Used as
/// the anchor for the [`LayoutResult::mm`] real-unit helper — every other
/// physical object in the scene (coins, candles, dishes, …) can be expressed
/// in true mm and we'll convert to world units via the current hand-slot width.
/// 25mm is near the short edge of a common Japanese tile; full-sized tiles are
/// ~26mm but 25 keeps the math clean.
pub const TILE_WIDTH_MM: f32 = 25.0;

/// Window-width fraction occupied by a single hand-tile slot. Falls out
/// of the hand layout: the hand strip is `(1 - 2*HAND_X_PAD_RATIO)` of
/// the window, divided into [`HAND_SIZE`] equal slots. This is also the
/// scale anchor for [`LayoutResult::mm`] in scenes that don't draw a hand
/// strip (shop, pick-blind, start screen).
pub const HAND_SLOT_W_RATIO: f32 = (1.0 - 2.0 * HAND_X_PAD_RATIO as f32) / HAND_SIZE as f32;

/// Score panel band height as a fraction of window height.
pub(crate) const SCORE_H_RATIO: f32 = 0.09;
/// Modifier strip height as a fraction of window height.
pub(crate) const MOD_H_RATIO: f32 = 0.05;
/// Horizontal inset (per side) of the hand strip relative to the window.
pub(crate) const HAND_X_PAD_RATIO: f32 = 0.16;

impl LayoutResult {
    /// Convert a length in **millimeters** to renderer world units. Uses
    /// the current hand-slot width as the reference for one mahjong tile
    /// (`TILE_WIDTH_MM`); when the scene has no hand strip the conversion
    /// falls back to a window-width-derived ratio that matches what the
    /// gameplay scene's hand tiles *would* be at the same window size,
    /// so physical object sizing stays consistent across scenes.
    pub fn mm(&self, n: f32) -> f32 {
        (self.hand_slot_w / TILE_WIDTH_MM) * n
    }

    /// Approximate top-band center when GLB score anchors are unavailable.
    pub fn fallback_score_center(&self) -> (f32, f32) {
        let band_h = (self.window_h * SCORE_H_RATIO).max(36.0);
        (self.window_w * 0.5, band_h * 0.5)
    }

    /// Approximate modifier-strip point when GLB popup sources are unavailable.
    pub fn fallback_modifier_point(&self, nx: f32, ny: f32) -> (f32, f32) {
        let score_h = (self.window_h * SCORE_H_RATIO).max(36.0);
        let mod_h = (self.window_h * MOD_H_RATIO).max(24.0);
        (self.window_w * nx, score_h + mod_h * ny)
    }

    pub(crate) fn hand_strip_origin(&self) -> (f32, f32, f32) {
        let score_h = (self.window_h * SCORE_H_RATIO).max(36.0);
        let mod_h = (self.window_h * MOD_H_RATIO).max(24.0);
        let x = self.window_w * HAND_X_PAD_RATIO;
        let w = self.hand_slot_w * self.hand_slot_count as f32;
        (x, score_h + mod_h, w)
    }
}

pub struct UiLayout {
    solve_cache: Option<(f32, f32, LayoutResult)>,
}

impl UiLayout {
    pub fn new() -> Self {
        Self { solve_cache: None }
    }

    pub fn solve(&mut self, width: f32, height: f32) -> LayoutResult {
        if let Some((w, h, cached)) = &self.solve_cache
            && *w == width
            && *h == height
        {
            return cached.clone();
        }

        let hand_x_pad = width * HAND_X_PAD_RATIO;
        let hand_strip_w = width - 2.0 * hand_x_pad;
        let hand_slot_w = hand_strip_w / HAND_SIZE as f32;
        let score_h = (height * SCORE_H_RATIO).max(36.0);
        let mod_h = (height * MOD_H_RATIO).max(24.0);

        let result = LayoutResult {
            window_w: width,
            window_h: height,
            hand_slot_w,
            hand_slot_h: height - (score_h + mod_h),
            hand_slot_count: HAND_SIZE,
        };
        self.solve_cache = Some((width, height, result.clone()));
        result
    }
}

impl Default for UiLayout {
    fn default() -> Self {
        Self::new()
    }
}
