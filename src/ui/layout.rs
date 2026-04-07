//! Cassowary layout: root window → score, modifier strip, hand strip + slots.

use cassowary::WeightedRelation::*;
use cassowary::strength::{REQUIRED, STRONG};
use cassowary::{Solver, Variable};

use crate::game::run::HAND_SIZE;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Solved UI layout for one frame.
#[derive(Debug)]
#[allow(dead_code)]
pub struct LayoutResult {
    pub window_w: f32,
    pub window_h: f32,
    pub score_panel: Rect,
    pub modifier_strip: Rect,
    pub relic_strip: Rect,
    pub hand_strip: Rect,
    /// `HAND_SIZE` equal-width slots inside hand (top-left, size).
    pub hand_slots: Vec<Rect>,
}

/// Score panel / modifier strip / relic strip are proportional to window height.
const SCORE_H_RATIO: f64 = 0.12; // 12% of window height (72px at 600px)
const MOD_H_RATIO: f64 = 0.08; //  8% of window height (48px at 600px)
const RELIC_H_RATIO: f64 = 0.12; // 12% of window height

pub struct UiLayout {
    solver: Solver,
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
    relic_left: Variable,
    relic_top: Variable,
    relic_w: Variable,
    relic_h: Variable,
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
        let relic_left = Variable::new();
        let relic_top = Variable::new();
        let relic_w = Variable::new();
        let relic_h = Variable::new();
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
                mod_top | EQ(REQUIRED) | score_top + score_h,
                relic_left | EQ(REQUIRED) | 0.0,
                relic_w | EQ(REQUIRED) | win_w,
                relic_top | EQ(REQUIRED) | mod_top + mod_h,
                hand_left | EQ(REQUIRED) | 0.0,
                hand_w | EQ(REQUIRED) | win_w,
                hand_top | EQ(REQUIRED) | relic_top + relic_h,
                hand_h | EQ(REQUIRED) | win_h - hand_top,
            ])
            .expect("layout constraints");

        let hs = HAND_SIZE as f64;
        solver
            .add_constraints(&[slot_w * hs | EQ(REQUIRED) | hand_w])
            .expect("slot width");

        solver.add_edit_variable(win_w, STRONG).expect("edit win_w");
        solver.add_edit_variable(win_h, STRONG).expect("edit win_h");
        solver
            .add_edit_variable(score_h, STRONG)
            .expect("edit score_h");
        solver.add_edit_variable(mod_h, STRONG).expect("edit mod_h");
        solver
            .add_edit_variable(relic_h, STRONG)
            .expect("edit relic_h");

        Self {
            solver,
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
            relic_left,
            relic_top,
            relic_w,
            relic_h,
            hand_left,
            hand_top,
            hand_w,
            hand_h,
            slot_w,
        }
    }

    pub fn solve(&mut self, width: f32, height: f32) -> LayoutResult {
        let sh = (height as f64 * SCORE_H_RATIO).max(36.0);
        let mh = (height as f64 * MOD_H_RATIO).max(24.0);
        let rh = (height as f64 * RELIC_H_RATIO).max(48.0);

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
        self.solver
            .suggest_value(self.relic_h, rh)
            .expect("suggest relic_h");

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
        let relic_strip = Rect {
            x: g(self.relic_left),
            y: g(self.relic_top),
            w: g(self.relic_w),
            h: g(self.relic_h),
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

        LayoutResult {
            window_w: ww,
            window_h: hh,
            score_panel,
            modifier_strip,
            relic_strip,
            hand_strip,
            hand_slots,
        }
    }
}

impl Default for UiLayout {
    fn default() -> Self {
        Self::new()
    }
}
