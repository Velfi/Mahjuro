//! Cassowary layout: root window → score, modifier strip, hand strip + slots.
//!
//! Active relics are **not** Cassowary rects: they render as a **horizontal 3D tray**
//! across the upper screen (`build_relic_tray_and_wind` in `gameplay/input_handler.rs`),
//! centered/clamped via [`crate::scenes::gameplay::glb_anchors`] when `gameplay.glb` is loaded.
//! The old `relic_strip` layout field is gone; `hand_top` sits directly below the
//! modifier strip.

use cassowary::WeightedRelation::*;
use cassowary::strength::{REQUIRED, STRONG};
use cassowary::{Solver, Variable};

use crate::game::game_mode::HAND_SIZE;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Window dimensions for widget layout helpers.
#[derive(Clone, Copy, Debug)]
pub struct ViewportCtx {
    pub window_w: f32,
    pub window_h: f32,
}

/// Solved UI layout for one frame.
#[derive(Clone, Debug)]
pub struct LayoutResult {
    pub window_w: f32,
    pub window_h: f32,
    pub score_panel: Rect,
    pub modifier_strip: Rect,
    pub hand_strip: Rect,
    /// `HAND_SIZE` equal-width slots inside hand (top-left, size).
    pub hand_slots: Vec<Rect>,
}

/// Real-world width of a Japanese mahjong tile, in millimeters. Used as
/// the anchor for the [`LayoutResult::mm`] / [`LayoutResult::m`] real-unit
/// helpers — every other physical object in the scene (coins, candles,
/// dishes, …) can be expressed in true mm/m and we'll convert to world
/// units via the current hand-slot width. 25mm is near the short edge of a
/// common Japanese tile; full-sized tiles are ~26mm but 25 keeps the math clean.
pub const TILE_WIDTH_MM: f32 = 25.0;

/// Window-width fraction occupied by a single hand-tile slot. Falls out
/// of the hand layout: the hand strip is `(1 - 2*HAND_X_PAD_RATIO)` of
/// the window, divided into [`HAND_SIZE`] equal slots. This is also the
/// scale anchor for [`LayoutResult::mm`] / [`LayoutResult::m`] in scenes
/// that don't draw a hand strip (shop, pick-blind, start screen).
pub const HAND_SLOT_W_RATIO: f32 = (1.0 - 2.0 * HAND_X_PAD_RATIO as f32) / HAND_SIZE as f32;

impl LayoutResult {
    /// Convert a length in **millimeters** to renderer world units. Uses
    /// the current hand-slot width as the reference for one mahjong tile
    /// (`TILE_WIDTH_MM`); when the scene has no hand strip the conversion
    /// falls back to a window-width-derived ratio that matches what the
    /// gameplay scene's hand tiles *would* be at the same window size,
    /// so physical object sizing stays consistent across scenes.
    pub fn mm(&self, n: f32) -> f32 {
        let tile_w = self
            .hand_slots
            .first()
            .map(|r| r.w)
            .unwrap_or(self.window_w * HAND_SLOT_W_RATIO);
        (tile_w / TILE_WIDTH_MM) * n
    }
}

/// Score panel and modifier strip are proportional to window height.
/// Relic medallions are separate 3D placement (horizontal tray), not these rects.
const SCORE_H_RATIO: f64 = 0.09; //  9% of window height (54px at 600px)
const MOD_H_RATIO: f64 = 0.05; //  5% of window height (30px at 600px)
/// Horizontal inset (per side) of the hand strip relative to the window.
/// Reduced from 22% → 16% to widen the tile rack; leaves margin for table framing.
const HAND_X_PAD_RATIO: f64 = 0.16;

pub struct UiLayout {
    solver: Solver,
    /// Last `(width, height)` → layout when identical next frame (continuous redraw).
    solve_cache: Option<(f32, f32, LayoutResult)>,
    win_w: Variable,
    win_h: Variable,
    score_left: Variable,
    score_top: Variable,
    score_w: Variable,
    score_h: Variable,
    mod_left: Variable,
    mod_top: Variable,
    mod_w: Variable,
    mod_h: Variable,
    hand_left: Variable,
    hand_top: Variable,
    hand_w: Variable,
    hand_h: Variable,
    /// Slot width inside hand (shared).
    slot_w: Variable,
}

impl UiLayout {
    pub fn new() -> Self {
        let mut solver = Solver::new();
        let win_w = Variable::new();
        let win_h = Variable::new();
        let score_left = Variable::new();
        let score_top = Variable::new();
        let score_w = Variable::new();
        let score_h = Variable::new();
        let mod_left = Variable::new();
        let mod_top = Variable::new();
        let mod_w = Variable::new();
        let mod_h = Variable::new();
        let hand_left = Variable::new();
        let hand_top = Variable::new();
        let hand_w = Variable::new();
        let hand_h = Variable::new();
        let slot_w = Variable::new();

        solver
            .add_constraints(&[
                win_w | GE(REQUIRED) | 320.0,
                win_h | GE(REQUIRED) | 240.0,
                score_left | EQ(REQUIRED) | 0.0,
                score_top | EQ(REQUIRED) | 0.0,
                score_w | EQ(REQUIRED) | win_w,
                mod_left | EQ(REQUIRED) | 0.0,
                mod_w | EQ(REQUIRED) | win_w,
                mod_top | EQ(REQUIRED) | (score_top + score_h),
                hand_left | EQ(REQUIRED) | (win_w * HAND_X_PAD_RATIO),
                hand_w | EQ(REQUIRED) | (win_w * (1.0 - 2.0 * HAND_X_PAD_RATIO)),
                hand_top | EQ(REQUIRED) | (mod_top + mod_h),
                hand_h | EQ(REQUIRED) | (win_h - hand_top),
            ])
            .expect("layout constraints");

        let hs = HAND_SIZE as f64;
        solver
            .add_constraints(&[(slot_w * hs) | EQ(REQUIRED) | hand_w])
            .expect("slot width");

        solver.add_edit_variable(win_w, STRONG).expect("edit win_w");
        solver.add_edit_variable(win_h, STRONG).expect("edit win_h");
        solver
            .add_edit_variable(score_h, STRONG)
            .expect("edit score_h");
        solver.add_edit_variable(mod_h, STRONG).expect("edit mod_h");

        Self {
            solver,
            solve_cache: None,
            win_w,
            win_h,
            score_left,
            score_top,
            score_w,
            score_h,
            mod_left,
            mod_top,
            mod_w,
            mod_h,
            hand_left,
            hand_top,
            hand_w,
            hand_h,
            slot_w,
        }
    }

    pub fn solve(&mut self, width: f32, height: f32) -> LayoutResult {
        if let Some((w, h, cached)) = &self.solve_cache
            && *w == width
            && *h == height
        {
            return cached.clone();
        }

        let sh = (height as f64 * SCORE_H_RATIO).max(36.0);
        let mh = (height as f64 * MOD_H_RATIO).max(24.0);
        self.solver
            .suggest_value(self.win_w, width as f64)
            .expect("suggest w");
        self.solver
            .suggest_value(self.win_h, height as f64)
            .expect("suggest h");
        self.solver
            .suggest_value(self.score_h, sh)
            .expect("suggest score_h");
        self.solver
            .suggest_value(self.mod_h, mh)
            .expect("suggest mod_h");

        let ww = width;
        let hh = height;
        let g = |v: Variable| self.solver.get_value(v) as f32;

        let score_panel = Rect {
            x: g(self.score_left),
            y: g(self.score_top),
            w: g(self.score_w),
            h: g(self.score_h),
        };
        let modifier_strip = Rect {
            x: g(self.mod_left),
            y: g(self.mod_top),
            w: g(self.mod_w),
            h: g(self.mod_h),
        };
        let hand_strip = Rect {
            x: g(self.hand_left),
            y: g(self.hand_top),
            w: g(self.hand_w),
            h: g(self.hand_h),
        };

        let sw = g(self.slot_w);
        let mut hand_slots = Vec::with_capacity(HAND_SIZE);
        let hx = hand_strip.x;
        let hy = hand_strip.y;
        let hsh = hand_strip.h;
        for i in 0..HAND_SIZE {
            hand_slots.push(Rect {
                x: hx + i as f32 * sw,
                y: hy,
                w: sw,
                h: hsh,
            });
        }

        let result = LayoutResult {
            window_w: ww,
            window_h: hh,
            score_panel,
            modifier_strip,
            hand_strip,
            hand_slots,
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
