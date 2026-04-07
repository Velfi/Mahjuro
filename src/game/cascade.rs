//! Scoring cascade: step-by-step reveal of a ScoreBreakdown with timing.
//!
//! The cascade progresses through phases:
//! 1. **ShowBase** — display the base points for a beat
//! 2. **ShowStep(i)** — reveal each relic/rule contribution one at a time
//! 3. **ShowTotal** — hold on the final total
//! 4. **Done** — cascade finished, input unblocked

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::core::scoring::{ScoreBreakdown, StepKind};

/// Tunable timing parameters for the scoring cascade animation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CascadeTuning {
    /// How long the base-points phase is displayed (ms).
    pub base_hold_ms: u64,
    /// How long each relic/rule step is displayed (ms).
    pub step_hold_ms: u64,
    /// How long the final total is held before unblocking (ms).
    pub total_hold_ms: u64,
    /// Duration for the score counter tick-up animation (ms).
    pub tick_duration_ms: u64,
    /// Lifetime of the discard departure animation (ms). Higher = slower float-away.
    #[serde(default = "default_depart_ms")]
    pub depart_lifetime_ms: u64,
    /// Duration for drawn tiles to settle into position (ms). Higher = slower slide-in.
    #[serde(default = "default_draw_ms")]
    pub draw_settle_ms: u64,
    /// Duration for sort/drag tile repositioning (ms). Higher = slower shuffle.
    #[serde(default = "default_sort_ms")]
    pub sort_settle_ms: u64,
}

fn default_depart_ms() -> u64 {
    700
}
fn default_draw_ms() -> u64 {
    500
}
fn default_sort_ms() -> u64 {
    400
}

impl Default for CascadeTuning {
    fn default() -> Self {
        Self {
            base_hold_ms: 600,
            step_hold_ms: 500,
            total_hold_ms: 900,
            tick_duration_ms: 350,
            depart_lifetime_ms: 700,
            draw_settle_ms: 500,
            sort_settle_ms: 400,
        }
    }
}

impl CascadeTuning {
    pub fn base_hold(&self) -> Duration {
        Duration::from_millis(self.base_hold_ms)
    }
    pub fn step_hold(&self) -> Duration {
        Duration::from_millis(self.step_hold_ms)
    }
    pub fn total_hold(&self) -> Duration {
        Duration::from_millis(self.total_hold_ms)
    }
    pub fn tick_duration(&self) -> Duration {
        Duration::from_millis(self.tick_duration_ms)
    }
}

#[derive(Clone, Debug)]
enum Phase {
    /// Showing base points.
    ShowBase,
    /// Showing relic/rule step at index `i`.
    ShowStep(usize),
    /// Holding on the final total.
    ShowTotal,
    /// Cascade complete.
    Done,
}

/// Active scoring cascade animation.
#[derive(Clone, Debug)]
pub struct ScoringCascade {
    pub breakdown: ScoreBreakdown,
    phase: Phase,
    phase_started: Instant,
    /// The round score *before* this hand was scored.
    pub score_before: u32,
    /// Points earned this hand.
    pub earned: u32,
    /// Timing parameters.
    tuning: CascadeTuning,
}

/// What the UI should display for the current cascade frame.
pub struct CascadeFrame {
    /// The displayed round score (ticking up).
    pub displayed_score: u32,
    /// Whether the cascade is still running (blocks input).
    pub active: bool,
    /// Index of the step that just appeared (for pulse animation). None if base or total.
    pub new_step_index: Option<usize>,
    /// Current chip pile, smoothly interpolated for the active phase.
    pub displayed_chips: i32,
    /// Current mult, smoothly interpolated for the active phase.
    pub displayed_mult: f64,
    /// The most recently fired step (source label + which axis it hit).
    /// `None` while still on the base/total beats.
    pub latest_step: Option<(String, StepKind)>,
    /// 0..1 progress through the current phase, used for pulse easing.
    pub phase_t: f32,
    /// Which axis (if any) just got an update — drives the bump animation.
    pub pulse_axis: Option<StepKind>,
}

impl ScoringCascade {
    pub fn with_tuning(
        breakdown: ScoreBreakdown,
        score_before: u32,
        earned: u32,
        tuning: CascadeTuning,
    ) -> Self {
        Self {
            breakdown,
            phase: Phase::ShowBase,
            phase_started: Instant::now(),
            score_before,
            earned,
            tuning,
        }
    }

    /// Advance the cascade state machine. Call once per frame.
    pub fn update(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.phase_started);
        match &self.phase {
            Phase::ShowBase => {
                if elapsed >= self.tuning.base_hold() {
                    if self.breakdown.steps.is_empty() {
                        self.phase = Phase::ShowTotal;
                    } else {
                        self.phase = Phase::ShowStep(0);
                    }
                    self.phase_started = now;
                }
            }
            Phase::ShowStep(i) => {
                let i = *i;
                if elapsed >= self.tuning.step_hold() {
                    if i + 1 < self.breakdown.steps.len() {
                        self.phase = Phase::ShowStep(i + 1);
                    } else {
                        self.phase = Phase::ShowTotal;
                    }
                    self.phase_started = now;
                }
            }
            Phase::ShowTotal => {
                if elapsed >= self.tuning.total_hold() {
                    self.phase = Phase::Done;
                    self.phase_started = now;
                }
            }
            Phase::Done => {}
        }
    }

    /// Is the cascade still animating?
    pub fn is_active(&self) -> bool {
        !matches!(self.phase, Phase::Done)
    }

    /// Skip to done (e.g. if player presses a key to skip).
    pub fn skip(&mut self) {
        self.phase = Phase::Done;
    }

    /// Build the current frame for rendering.
    pub fn frame(&self, now: Instant) -> CascadeFrame {
        let elapsed = now.saturating_duration_since(self.phase_started);

        // How far through the score tick are we?
        let tick_t = (elapsed.as_secs_f32() / self.tuning.tick_duration().as_secs_f32()).min(1.0);

        // Phase-relative 0..1 (used by the renderer for the pulse envelope).
        let phase_t = match &self.phase {
            Phase::ShowBase => {
                (elapsed.as_secs_f32() / self.tuning.base_hold().as_secs_f32()).min(1.0)
            }
            Phase::ShowStep(_) => {
                (elapsed.as_secs_f32() / self.tuning.step_hold().as_secs_f32()).min(1.0)
            }
            Phase::ShowTotal => {
                (elapsed.as_secs_f32() / self.tuning.total_hold().as_secs_f32()).min(1.0)
            }
            Phase::Done => 1.0,
        };

        // Determine the ticking score target and which step (if any) just fired.
        let (score_target, new_step_index) = match &self.phase {
            Phase::ShowBase => {
                let target = self.score_before + self.breakdown.base_chips.max(0) as u32;
                (lerp_u32(self.score_before, target, tick_t), None)
            }
            Phase::ShowStep(i) => {
                let i = *i;
                let running = self.breakdown.steps[i].running_total.max(0) as u32;
                let prev_running = if i > 0 {
                    self.breakdown.steps[i - 1].running_total.max(0) as u32
                } else {
                    self.breakdown.base_chips.max(0) as u32
                };
                let from = self.score_before + prev_running;
                let to = self.score_before + running;
                (lerp_u32(from, to, tick_t), Some(i))
            }
            Phase::ShowTotal | Phase::Done => {
                let total = self.score_before + self.earned;
                (total, None)
            }
        };

        // ── Two-axis chips/mult readout ────────────────────────────────────
        //
        // Compute the *currently displayed* chip pile and mult value, smoothly
        // interpolating across the active phase. This lets the renderer draw
        // them as two big counters that tick up rather than as a text scroll.
        let (displayed_chips, displayed_mult, latest_step, pulse_axis) = match &self.phase {
            Phase::ShowBase => {
                // Tick chips from 0 → base_chips. Mult sits at ×1.
                let to = self.breakdown.base_chips as f64;
                let chips = (to * tick_t as f64).round() as i32;
                (chips, 1.0, None, None)
            }
            Phase::ShowStep(i) => {
                let i = *i;
                let step = &self.breakdown.steps[i];
                let (prev_chips, prev_mult) = if i > 0 {
                    let p = &self.breakdown.steps[i - 1];
                    (p.running_chips as f64, p.running_mult)
                } else {
                    (self.breakdown.base_chips as f64, 1.0)
                };
                let to_chips = step.running_chips as f64;
                let to_mult = step.running_mult;
                let t = tick_t as f64;
                let chips = (prev_chips + (to_chips - prev_chips) * t).round() as i32;
                let mult = prev_mult + (to_mult - prev_mult) * t;
                let pulse = if step.kind == StepKind::Final {
                    None // final beat handled by total color
                } else {
                    Some(step.kind)
                };
                (chips, mult, Some((step.source.clone(), step.kind)), pulse)
            }
            Phase::ShowTotal | Phase::Done => {
                // Hold the final values.
                let chips = self
                    .breakdown
                    .steps
                    .last()
                    .map(|s| s.running_chips)
                    .unwrap_or(self.breakdown.base_chips);
                let mult = self
                    .breakdown
                    .steps
                    .last()
                    .map(|s| s.running_mult)
                    .unwrap_or(1.0);
                (chips, mult, None, None)
            }
        };

        CascadeFrame {
            displayed_score: score_target,
            active: self.is_active(),
            new_step_index,
            displayed_chips,
            displayed_mult,
            latest_step,
            phase_t,
            pulse_axis,
        }
    }
}

fn lerp_u32(from: u32, to: u32, t: f32) -> u32 {
    if to >= from {
        from + ((to - from) as f32 * t) as u32
    } else {
        from - ((from - to) as f32 * t) as u32
    }
}
