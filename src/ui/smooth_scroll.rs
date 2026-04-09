//! Smooth-scroll helper for entry-unit scroll panels.
//!
//! Wraps a float scroll target with a visual position that
//! exponentially decays toward it each frame.  Accepts raw
//! fractional scroll deltas so trackpad momentum events aren't
//! rounded away.  Because overlay `draw()` methods take `&self`,
//! all fields use `Cell`.

use std::cell::Cell;
use std::time::Instant;

/// Exponential-decay speed for the visual → target chase.
/// Higher = tighter tracking.  At 14 the lag is just perceptible
/// enough to read as "smooth" without feeling sluggish.
const SMOOTH_SPEED: f32 = 14.0;

/// Snap threshold — when visual is this close to target, lock on
/// to avoid perpetual sub-pixel drift.
const SNAP_THRESHOLD: f32 = 0.005;

/// Smooth-scrolling state for panels that scroll in entry units.
pub struct SmoothScroll {
    /// Scroll target in entry units (float — accumulates raw deltas).
    target: Cell<f32>,
    /// Maximum legal target value.  Updated each draw when the
    /// window resizes.
    max: Cell<f32>,
    /// Smoothly interpolated position that the renderer reads.
    visual: Cell<f32>,
    /// Timestamp of the last `tick()` call, used to derive dt.
    last_tick: Cell<Instant>,
}

impl SmoothScroll {
    pub fn new() -> Self {
        Self {
            target: Cell::new(0.0),
            max: Cell::new(0.0),
            visual: Cell::new(0.0),
            last_tick: Cell::new(Instant::now()),
        }
    }

    // ── Getters ──────────────────────────────────────────────────────

    pub fn target(&self) -> f32 {
        self.target.get()
    }

    pub fn max(&self) -> f32 {
        self.max.get()
    }

    // ── Input methods ────────────────────────────────────────────────

    /// Add a raw scroll delta (positive = scroll *down* / content moves
    /// up).  This is the primary entry point for mouse-wheel and
    /// trackpad input — it preserves fractional deltas so macOS
    /// momentum scrolling works naturally.
    pub fn scroll_by(&self, delta: f32) {
        let t = (self.target.get() + delta).clamp(0.0, self.max.get());
        self.target.set(t);
    }

    /// Step by exactly one entry in the given direction.  Rounds the
    /// current target to the nearest integer first so repeated
    /// keyboard / d-pad presses land on clean entry boundaries.
    pub fn step(&self, direction: i32) {
        let rounded = self.target.get().round();
        let t = (rounded + direction as f32).clamp(0.0, self.max.get());
        self.target.set(t);
    }

    /// Set an absolute target (e.g. TOC link click).
    pub fn set_target(&self, t: f32) {
        self.target.set(t.clamp(0.0, self.max.get()));
    }

    /// Jump instantly — no animation.  Used when re-opening an overlay.
    pub fn jump(&self, t: f32) {
        let clamped = t.clamp(0.0, self.max.get());
        self.target.set(clamped);
        self.visual.set(clamped);
    }

    /// Update the maximum and clamp target + visual if needed.
    pub fn set_max(&self, m: u32) {
        let m = m as f32;
        self.max.set(m);
        if self.target.get() > m {
            self.target.set(m);
        }
        if self.visual.get() > m {
            self.visual.set(m);
        }
    }

    // ── Per-frame update ─────────────────────────────────────────────

    /// Advance the visual position toward the target.  Call once per
    /// frame (typically at the top of `draw`).  Returns the current
    /// smooth scroll position as a float in entry units.
    pub fn tick(&self) -> f32 {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_tick.get())
            .as_secs_f32()
            .min(0.1); // cap so a long pause doesn't teleport
        self.last_tick.set(now);

        let goal = self.target.get();
        let mut v = self.visual.get();

        if (v - goal).abs() < SNAP_THRESHOLD {
            v = goal;
        } else {
            v += (goal - v) * (1.0 - (-SMOOTH_SPEED * dt).exp());
        }

        self.visual.set(v);
        v
    }
}
